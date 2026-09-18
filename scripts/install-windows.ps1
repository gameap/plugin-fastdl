#Requires -Version 5.1

<#
GameAP FastDL installation script for Windows.

Resolves the latest stable gameap-fastdl release, verifies its published SHA256
before it is ever executed, installs it into the private plugin
directory and registers the Windows service that serves FastDL content.

Requires an administrative account. The service runs as LocalSystem, and the
plugin directory is closed to every other account: the panel keeps writing
config.json and servers.d\*.json into it through gameap-daemon, whose identity
is granted access alongside SYSTEM and Administrators.

-ConfigPath must live in -InstallDir: gameap-fastdl resolves a relative
servers_dir and cache_dir against the configuration file's own directory, so
elsewhere the service would use directories this installation never prepares.

Re-running the script updates an existing installation. The previous executable
and service definition are kept until the new service is confirmed running, and
restored when it is not. A re-run that resolves to the same executable and the
same service definition leaves the running service alone.

Invoked by the panel's FastDL plugin as a daemon task:
  powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File
    "{node_work_path}\.plugins\i3z7ix336msd4\install-windows.ps1"
    -InstallDir "{node_work_path}\.plugins\i3z7ix336msd4"
    -ConfigPath "{node_work_path}\.plugins\i3z7ix336msd4\config.json"
#>

param(
    [string]$DownloadUrl = "",
    [string]$Sha256 = "",
    [string]$InstallDir = "",
    [string]$ConfigPath = "",
    [switch]$FixAcl,
    [switch]$Check,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
# Invoke-WebRequest renders a progress bar on every chunk, which dominates the
# runtime of a download on Windows PowerShell 5.1.
$ProgressPreference = "SilentlyContinue"

# The panel reads this script's output from a daemon task, where a PowerShell
# exception blob is far less useful than the message alone.
trap {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}

$COMPONENT = "gameap-fastdl"
$GITHUB_REPO = "gameap/gameap-fastdl"
$SERVICE_NAME = "gameap-fastdl"
$SERVICE_DISPLAY_NAME = "GameAP FastDL"
$SERVICE_DESCRIPTION = "Serves GoldSource and Source game content for FastDL downloads."
$SERVERS_SUBDIR = "servers.d"
$CACHE_SUBDIR = "cache"

# The same cap the Linux installer passes to curl.
$MAX_DOWNLOAD_BYTES = 134217728
$MAX_REDIRECTS = 5
$DOWNLOAD_TIMEOUT_SECONDS = 600

# The SCM reports Running as soon as the process reports it, which says nothing
# about an executable that dies on its listen address; the start has to hold.
$SERVICE_WAIT_SECONDS = 30
$SETTLE_SECONDS = 5
$STALE_STAGING_MINUTES = 60

# Mirrors Restart=on-failure / RestartSec=3 in the systemd unit.
$RECOVERY_RESET_SECONDS = 60
$RECOVERY_ACTIONS = "restart/5000/restart/10000/restart/30000"

function Show-Help {
    Write-Host @"
GameAP FastDL installation script for Windows

Usage: powershell -NoProfile -ExecutionPolicy Bypass -File install-windows.ps1 [options]

Required:
  -InstallDir DIR      Private plugin directory holding the executable,
                       $SERVERS_SUBDIR\ and $CACHE_SUBDIR\ (the panel passes
                       <work path>\.plugins\i3z7ix336msd4)
  -ConfigPath FILE     FastDL configuration file, inside -InstallDir; the panel
                       writes it before this script runs. Defaults to
                       <InstallDir>\config.json

Other:
  -DownloadUrl URL     Override the latest stable GitHub release with an explicit
                       HTTPS executable URL; requires -Sha256
  -Sha256 HEX          Expected SHA256 for -DownloadUrl (64 hex characters).
                       Without this pair, the release asset's matching .sha256
                       sidecar is required
  -FixAcl              Re-apply the directory permissions to files that already
                       exist. Needed once on a node installed by a release that
                       set permissions per file instead of by inheritance; the
                       cache directory is skipped
  -Check               Report the installed version and service state and exit
                       without changing anything (exit 1 when unhealthy). The
                       paths are read from the installed service when the
                       options above are omitted
  -Help                Show this help

Re-running with the same -Sha256 keeps the installed executable: the download is
skipped, and the service is restarted only when the executable or the service
definition actually changed. The service runs as LocalSystem, starts
automatically and is restarted by the SCM after a crash. No firewall rule is
created - open the configured listen port yourself.
"@
}

if ($Help) {
    Show-Help
    exit 0
}

# Windows PowerShell 5.1 negotiates SSL3/TLS 1.0 by default, which most CDNs
# refuse. 3072 is Tls12 as a literal: on .NET 4.0-era systems the named enum
# member does not exist and referencing it fails to parse. -bor keeps whatever
# the administrator has already enabled, including TLS 1.3. On PowerShell 7 the
# property does not affect HttpClient at all, so it is skipped.
if ($PSVersionTable.PSEdition -eq "Desktop") {
    try {
        if ([Net.ServicePointManager]::SecurityProtocol -ne 0) {
            [Net.ServicePointManager]::SecurityProtocol =
                [Net.ServicePointManager]::SecurityProtocol -bor 3072
        }
    } catch {
        Write-Warning "Could not enable TLS 1.2; the download will most likely fail."
    }
}

function Exit-WithError {
    param([string]$Message)

    [Console]::Error.WriteLine($Message)
    exit 1
}

# Native executables do not raise PowerShell errors, so $ErrorActionPreference
# never sees them - every external call has to be checked explicitly.
function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$ErrorMessage,
        [switch]$IgnoreExitCode
    )

    # Under `$ErrorActionPreference = "Stop"` a native command that merely writes
    # to stderr raises a terminating error as soon as 2>&1 wraps the line in an
    # ErrorRecord - before the exit code can be looked at. gameap-fastdl logs to
    # stderr, so without this every failure would surface as a PowerShell blob.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $FilePath @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }

    if ($exitCode -ne 0 -and -not $IgnoreExitCode) {
        if ($output) {
            Write-Host ($output | Out-String).TrimEnd()
        }
        $message = if ($ErrorMessage) { $ErrorMessage } else { "$FilePath failed" }
        throw "${message} (exit code ${exitCode})"
    }

    return [pscustomobject]@{ ExitCode = $exitCode; Output = $output }
}

# ---------------------------------------------------------------------------
# Option and path validation
#
# The SCM stores a service's ImagePath as REG_EXPAND_SZ, so a % in a path is
# expanded at service start and would point the service somewhere else; a double
# quote would end the binPath argument early. Both are refused rather than
# escaped, so every later comparison can use the raw string.

function Assert-PlainAbsolutePath {
    param([string]$Name, [string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        Exit-WithError "$Name is required"
    }
    if (-not [IO.Path]::IsPathRooted($Path)) {
        Exit-WithError "$Name must be an absolute path, got '$Path'"
    }
    if ($Path -match '["%]' -or $Path -match '[\x00-\x1f]') {
        Exit-WithError "$Name must not contain double quotes or percent signs: '$Path'"
    }
}

# A junction or symbolic link anywhere in the chain would let whoever controls
# it redirect a privileged write outside the private directory.
function Assert-NoReparsePath {
    param([string]$Path)

    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                Exit-WithError "Installation paths must not contain reparse points: $current"
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
    }
}

function Test-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)

    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# ---------------------------------------------------------------------------
# Permissions
#
# The protected access rules are applied to the plugin directory and inherited
# by everything inside it, including a binary moved in from the staging
# directory. An earlier release instead walked the tree and protected each file
# separately, which disabled inheritance on every one of them and made the walk
# grow with the cache directory; -FixAcl repairs those nodes once.

function New-PrivateAcl {
    param([bool]$Directory, [string]$InstallerSid)

    $acl = if ($Directory) {
        New-Object Security.AccessControl.DirectorySecurity
    } else {
        New-Object Security.AccessControl.FileSecurity
    }

    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner((New-Object Security.Principal.SecurityIdentifier("S-1-5-32-544")))

    $inherit = if ($Directory) { "ContainerInherit,ObjectInherit" } else { "None" }
    foreach ($sidValue in @("S-1-5-18", "S-1-5-32-544", $InstallerSid)) {
        $sid = New-Object Security.Principal.SecurityIdentifier($sidValue)
        $rule = New-Object Security.AccessControl.FileSystemAccessRule(
            $sid, "FullControl", $inherit, "None", "Allow")
        $acl.AddAccessRule($rule)
    }

    return $acl
}

# Set-Acl on a directory propagates inheritance across the whole subtree even
# when nothing changed, so the common case is a single read and no write.
function Protect-PrivatePath {
    param([string]$Path, [string]$InstallerSid)

    $item = Get-Item -LiteralPath $Path -Force
    $isDirectory = $item.PSIsContainer
    $desired = New-PrivateAcl -Directory $isDirectory -InstallerSid $InstallerSid

    $current = Get-Acl -LiteralPath $Path
    $currentRules = @($current.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]) |
        ForEach-Object { "$($_.IdentityReference)|$($_.FileSystemRights)|$($_.AccessControlType)|$($_.InheritanceFlags)" } |
        Sort-Object)
    $desiredRules = @($desired.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]) |
        ForEach-Object { "$($_.IdentityReference)|$($_.FileSystemRights)|$($_.AccessControlType)|$($_.InheritanceFlags)" } |
        Sort-Object)

    if ($current.AreAccessRulesProtected -and
        ($currentRules -join "`n") -eq ($desiredRules -join "`n")) {
        return
    }

    Set-Acl -LiteralPath $Path -AclObject $desired
}

function Repair-InheritedAcl {
    param([string]$InstallDir, [string]$ConfigPath, [string]$Binary)

    # The cache directory is deliberately left out: stale rules there are a
    # performance artefact, not an exposure, and every file in it is derived.
    $targets = @($ConfigPath, $Binary, (Join-Path $InstallDir $SERVERS_SUBDIR))
    foreach ($target in $targets) {
        if (-not (Test-Path -LiteralPath $target)) { continue }
        try {
            $acl = Get-Acl -LiteralPath $target
            $acl.SetAccessRuleProtection($false, $false)
            Set-Acl -LiteralPath $target -AclObject $acl
        } catch {
            Write-Warning "Could not restore inheritance on ${target}: $($_.Exception.Message)"
        }
    }

    foreach ($child in Get-ChildItem -LiteralPath (Join-Path $InstallDir $SERVERS_SUBDIR) -Force -ErrorAction SilentlyContinue) {
        try {
            $acl = Get-Acl -LiteralPath $child.FullName
            $acl.SetAccessRuleProtection($false, $false)
            Set-Acl -LiteralPath $child.FullName -AclObject $acl
        } catch {
            Write-Warning "Could not restore inheritance on $($child.FullName): $($_.Exception.Message)"
        }
    }
}

# ---------------------------------------------------------------------------
# Download
#
# Redirects are followed by hand so every hop can be re-checked for HTTPS and
# credentials; AllowAutoRedirect would follow a plain-HTTP Location silently.

function Get-FileSha256 {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return "" }

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-HttpsUri {
    param([Uri]$Uri, [string]$What)

    if ($Uri.Scheme -ne "https") {
        throw "$What must use HTTPS, got '$($Uri.OriginalString)'"
    }
    if ($Uri.UserInfo) {
        throw "$What must not carry credentials"
    }
}

function Save-Download {
    param([Uri]$Uri, [string]$Destination, [int]$TimeoutSeconds, [long]$MaxBytes = $MAX_DOWNLOAD_BYTES)

    Add-Type -AssemblyName System.Net.Http

    $deadline = [Diagnostics.Stopwatch]::StartNew()
    $handler = New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $client = New-Object Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromSeconds($TimeoutSeconds)
    $client.DefaultRequestHeaders.Add("User-Agent", "gameap-fastdl-installer")

    try {
        $current = $Uri
        for ($redirect = 0; $redirect -le $MAX_REDIRECTS; $redirect++) {
            Assert-HttpsUri -Uri $current -What "The download URL"

            $response = $client.GetAsync(
                $current, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            try {
                if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                    if (-not $response.Headers.Location) {
                        throw "The server answered $([int]$response.StatusCode) without a Location header"
                    }
                    if ($redirect -eq $MAX_REDIRECTS) {
                        throw "The download followed more than $MAX_REDIRECTS redirects"
                    }
                    $current = New-Object Uri($current, $response.Headers.Location)
                    continue
                }

                $response.EnsureSuccessStatusCode() | Out-Null
                if ($response.Content.Headers.ContentLength -gt $MaxBytes) {
                    throw "The download declares $($response.Content.Headers.ContentLength) bytes, over the $MaxBytes byte limit"
                }

                $stream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $file = [IO.File]::Open($Destination, [IO.FileMode]::Create)
                try {
                    $buffer = New-Object byte[] 65536
                    $total = 0L
                    while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $total += $read
                        # HttpClient.Timeout only covers the response headers
                        # once ResponseHeadersRead is used, so a stalled body
                        # would otherwise hang until the panel gives up.
                        if ($deadline.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
                            throw "The download did not finish within $TimeoutSeconds seconds"
                        }
                        # A chunked response declares no length, so the cap has
                        # to be enforced while reading as well.
                        if ($total -gt $MaxBytes) {
                            throw "The download is over the $MaxBytes byte limit"
                        }
                        $file.Write($buffer, 0, $read)
                    }
                } finally {
                    $file.Dispose()
                    $stream.Dispose()
                }
                return
            } finally {
                $response.Dispose()
            }
        }
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
}

function Get-ReleaseArchitecture {
    $architecture = $env:PROCESSOR_ARCHITECTURE
    if ($env:PROCESSOR_ARCHITEW6432) { $architecture = $env:PROCESSOR_ARCHITEW6432 }
    switch ($architecture) {
        "AMD64" { return "amd64" }
        "ARM64" { return "arm64" }
        default { throw "Only amd64 and arm64 releases are available; this node reports '$architecture'." }
    }
}

function Get-ReleaseTagFromUri {
    param([string]$ReleaseUri)

    $prefix = "https://github.com/$GITHUB_REPO/releases/tag/"
    if (-not $ReleaseUri.StartsWith($prefix, [StringComparison]::Ordinal)) {
        throw "No stable $COMPONENT release is published."
    }
    $tag = $ReleaseUri.Substring($prefix.Length)
    if ($tag -notmatch '\A[A-Za-z0-9][A-Za-z0-9._+-]*\z' -or $tag.Length -gt 128) {
        throw "The latest release has an invalid tag."
    }
    return $tag
}

function Get-LatestReleaseTag {
    Add-Type -AssemblyName System.Net.Http
    $handler = New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $client = New-Object Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(60)
    $client.DefaultRequestHeaders.Add("User-Agent", "gameap-fastdl-installer")
    $current = [Uri]"https://github.com/$GITHUB_REPO/releases/latest"
    try {
        for ($redirect = 0; $redirect -le $MAX_REDIRECTS; $redirect++) {
            Assert-HttpsUri -Uri $current -What "The release URL"
            $request = New-Object Net.Http.HttpRequestMessage([Net.Http.HttpMethod]::Head, $current)
            $response = $null
            try {
                $response = $client.SendAsync($request).GetAwaiter().GetResult()
                if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                    if (-not $response.Headers.Location -or $redirect -eq $MAX_REDIRECTS) {
                        throw "Could not resolve the latest stable $COMPONENT release."
                    }
                    $current = New-Object Uri($current, $response.Headers.Location)
                    continue
                }
                $response.EnsureSuccessStatusCode() | Out-Null
                return Get-ReleaseTagFromUri -ReleaseUri $current.AbsoluteUri
            } finally {
                if ($response) { $response.Dispose() }
                $request.Dispose()
            }
        }
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
}

function Read-ReleaseChecksum {
    param([string]$Content, [string]$Asset)

    $lines = @($Content -split '\r?\n' | Where-Object { $_.Trim() })
    if ($lines.Count -ne 1 -or $lines[0] -notmatch '\A([a-fA-F0-9]{64})(?:[ \t]+\*?([A-Za-z0-9._+-]+))?[ \t]*\z') {
        throw "Invalid SHA256 sidecar for $Asset."
    }
    $digest = $Matches[1]
    if ($Matches[2] -and $Matches[2] -cne $Asset) {
        throw "The SHA256 sidecar names a different release asset."
    }
    return $digest.ToLowerInvariant()
}

function Resolve-Release {
    $architecture = Get-ReleaseArchitecture
    $tag = Get-LatestReleaseTag
    $asset = "$COMPONENT-$tag-windows-$architecture.exe"
    $url = "https://github.com/$GITHUB_REPO/releases/download/$tag/$asset"
    $checksumPath = [IO.Path]::GetTempFileName()
    try {
        Save-Download -Uri "$url.sha256" -Destination $checksumPath -TimeoutSeconds 60 -MaxBytes 4096
        $digest = Read-ReleaseChecksum -Content ([IO.File]::ReadAllText($checksumPath)) -Asset $asset
    } finally {
        Remove-Item -LiteralPath $checksumPath -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Selected $COMPONENT $tag for windows/$architecture."
    return [pscustomobject]@{ DownloadUrl = $url; Sha256 = $digest }
}

function Assert-DownloadOptions {
    param([string]$Url, [string]$Digest)

    if ([bool]$Url -xor [bool]$Digest) {
        throw "-DownloadUrl and -Sha256 must be supplied together."
    }
    if ($Url) {
        if ($Digest -notmatch '\A[a-fA-F0-9]{64}\z') {
            throw "-Sha256 must be 64 hexadecimal characters."
        }
        $parsedUri = $null
        if (-not [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref]$parsedUri)) {
            throw "-DownloadUrl is not a valid absolute URL."
        }
        Assert-HttpsUri -Uri $parsedUri -What "-DownloadUrl"
    }
}

# ---------------------------------------------------------------------------
# Service handling
#
# Every state query goes through CIM rather than Get-Service: a ServiceController
# left undisposed keeps an SCM handle open, which is what turns a delete into a
# service stuck "marked for deletion".

function Get-ServiceInfo {
    param([string]$Name)

    return Get-CimInstance -ClassName Win32_Service -Filter "Name='$Name'" -ErrorAction SilentlyContinue
}

function Get-ServiceState {
    param([string]$Name)

    $info = Get-ServiceInfo -Name $Name
    if (-not $info) { return "not installed" }

    return $info.State
}

function Wait-ServiceState {
    param([string]$Name, [string]$State, [int]$TimeoutSeconds)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if ((Get-ServiceState -Name $Name) -eq $State) { return $true }
        Start-Sleep -Milliseconds 500
    }

    return (Get-ServiceState -Name $Name) -eq $State
}

function Stop-ServiceAndWait {
    param([string]$Name, [int]$TimeoutSeconds)

    $info = Get-ServiceInfo -Name $Name
    if (-not $info -or $info.State -eq "Stopped") { return }

    # Stop-Service on a service that is already stopping throws, because the
    # SCM reports it as unable to stop; waiting is the whole job there.
    if ($info.State -ne "Stop Pending") {
        Stop-Service -Name $Name -Force -ErrorAction SilentlyContinue
    }

    if (Wait-ServiceState -Name $Name -State "Stopped" -TimeoutSeconds $TimeoutSeconds) { return }

    # FastDL serves static files and holds no state worth draining, so a service
    # wedged in Stop Pending is killed rather than left blocking every future
    # upgrade of this node.
    $info = Get-ServiceInfo -Name $Name
    if ($info -and $info.ProcessId -gt 0) {
        Write-Warning "$Name did not stop within $TimeoutSeconds seconds; terminating process $($info.ProcessId)."
        Stop-Process -Id $info.ProcessId -Force -ErrorAction SilentlyContinue
        Wait-ServiceState -Name $Name -State "Stopped" -TimeoutSeconds 10 | Out-Null
    }
}

# The SCM reports Stopped before the image is always unmapped, and a virus
# scanner may hold a freshly written file too; replacing the executable while it
# is still locked is the most common way this script fails.
function Wait-FileUnlocked {
    param([string]$Path, [int]$TimeoutSeconds)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $true }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $handle = [IO.File]::Open($Path, "Open", "Write", "None")
            $handle.Dispose()
            return $true
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }

    return $false
}

function Remove-ServiceAndWait {
    param([string]$Name)

    # 1060 is "service does not exist" and 1072 "already marked for deletion";
    # both mean the service is on its way out, which is the requested end state.
    $result = Invoke-NativeCommand -FilePath "sc.exe" -Arguments @("delete", $Name) -IgnoreExitCode
    if ($result.ExitCode -notin @(0, 1060, 1072)) {
        Write-Warning "sc.exe delete $Name failed with exit code $($result.ExitCode)."
    }

    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        if (-not (Get-ServiceInfo -Name $Name)) { return }
        Start-Sleep -Milliseconds 500
    }
}

function Get-ServiceCommand {
    param([string]$Binary, [string]$ConfigPath)

    return '"' + $Binary + '" service --config "' + $ConfigPath + '"'
}

function Set-ServiceDefinition {
    param([string]$Name, [string]$Command, [bool]$Exists)

    if ($Exists) {
        Invoke-NativeCommand -FilePath "sc.exe" `
            -Arguments @("config", $Name, "binPath=", $Command, "start=", "auto") `
            -ErrorMessage "Unable to update the $Name service configuration" | Out-Null
    } else {
        New-Service -Name $Name -DisplayName $SERVICE_DISPLAY_NAME `
            -BinaryPathName $Command -StartupType Automatic | Out-Null
    }

    Invoke-NativeCommand -FilePath "sc.exe" `
        -Arguments @("description", $Name, $SERVICE_DESCRIPTION) -IgnoreExitCode | Out-Null

    # sc.exe quoting of an embedded-quote value is fragile enough to be worth
    # reading back rather than trusting the exit code.
    $info = Get-ServiceInfo -Name $Name
    if (-not $info) {
        throw "The $Name service is missing after configuring it"
    }
    if ($info.PathName -ne $Command) {
        throw "The $Name service command is '$($info.PathName)' instead of '$Command'"
    }
}

function Set-ServiceRecovery {
    param([string]$Name)

    Invoke-NativeCommand -FilePath "sc.exe" `
        -Arguments @("failure", $Name, "reset=", "$RECOVERY_RESET_SECONDS", "actions=", $RECOVERY_ACTIONS) `
        -IgnoreExitCode | Out-Null

    # Without failureflag the SCM only acts when the service dies without
    # reporting a stop, and gameap-fastdl reports one on a clean failure.
    Invoke-NativeCommand -FilePath "sc.exe" `
        -Arguments @("failureflag", $Name, "1") -IgnoreExitCode | Out-Null
}

function Clear-ServiceRecovery {
    param([string]$Name)

    Invoke-NativeCommand -FilePath "sc.exe" `
        -Arguments @("failure", $Name, "reset=", "0", "actions=", "") -IgnoreExitCode | Out-Null
}

function Get-ServiceEventLog {
    param([string]$Name)

    try {
        Get-WinEvent -MaxEvents 15 -FilterHashtable @{
            LogName = "System"; ProviderName = "Service Control Manager"
        } -ErrorAction SilentlyContinue |
            Where-Object { $_.Message -match [regex]::Escape($Name) } |
            ForEach-Object { "  $($_.TimeCreated): $($_.Message)" }
    } catch {
        return @()
    }
}

function Write-ServiceFailure {
    param([string]$Name, [string]$Binary, [string]$ConfigPath)

    [Console]::Error.WriteLine("--- Service Control Manager events for $Name ---")
    foreach ($line in (Get-ServiceEventLog -Name $Name)) {
        [Console]::Error.WriteLine($line)
    }
    # The SCM discards the service's stderr, so running it in the foreground is
    # the only way to see why it exits. 'serve' rather than 'service': the
    # service entry point refuses to start outside the SCM.
    [Console]::Error.WriteLine("Run it in the foreground to see why it exits:")
    [Console]::Error.WriteLine("  & '$Binary' serve --config '$ConfigPath'")
}

# ---------------------------------------------------------------------------
# Installed state and rollback

function Get-InstallationState {
    param([string]$Name, [string]$Binary)

    $info = Get-ServiceInfo -Name $Name

    return [pscustomobject]@{
        ServiceExists    = [bool]$info
        State            = if ($info) { $info.State } else { "" }
        WasRunning       = [bool]($info -and $info.State -in @("Running", "Start Pending"))
        StartMode        = if ($info) { $info.StartMode } else { "" }
        DelayedAutoStart = [bool]($info -and $info.DelayedAutoStart)
        PathName         = if ($info) { $info.PathName } else { "" }
        DisplayName      = if ($info) { $info.DisplayName } else { "" }
        StartName        = if ($info) { $info.StartName } else { "" }
        HadBinary        = Test-Path -LiteralPath $Binary -PathType Leaf
        InstalledSha256  = Get-FileSha256 -Path $Binary
        BinaryReplaced   = $false
        ServiceChanged   = $false
        Changed          = $false
    }
}

# Never throws: the top-level trap fires at script scope, so an exception raised
# here would abandon the remaining restore steps and replace the original error
# with whatever the rollback tripped over.
function Restore-PreviousInstallation {
    param($State, [string]$Staging, [string]$Binary, [string]$Name)

    [Console]::Error.WriteLine("Restoring the previous $COMPONENT...")

    try {
        Clear-ServiceRecovery -Name $Name
    } catch {
        Write-Warning "Could not clear the recovery actions: $($_.Exception.Message)"
    }

    try {
        Stop-ServiceAndWait -Name $Name -TimeoutSeconds 30
    } catch {
        Write-Warning "Could not stop $Name before restoring it: $($_.Exception.Message)"
    }

    if ($State.BinaryReplaced) {
        try {
            Wait-FileUnlocked -Path $Binary -TimeoutSeconds 15 | Out-Null
            $previous = Join-Path $Staging "previous.exe"
            if ($State.HadBinary -and (Test-Path -LiteralPath $previous)) {
                Move-Item -LiteralPath $previous -Destination $Binary -Force
            } elseif (-not $State.HadBinary) {
                Remove-Item -LiteralPath $Binary -Force -ErrorAction SilentlyContinue
            }
        } catch {
            Write-Warning "Could not restore the previous executable: $($_.Exception.Message)"
        }
    }

    if ($State.ServiceChanged) {
        try {
            if (-not $State.ServiceExists) {
                Remove-ServiceAndWait -Name $Name
            } else {
                $startMode = switch ($State.StartMode) {
                    "Auto"     { "auto" }
                    "Disabled" { "disabled" }
                    default    { "demand" }
                }
                Invoke-NativeCommand -FilePath "sc.exe" `
                    -Arguments @("config", $Name, "binPath=", $State.PathName, "start=", $startMode) `
                    -IgnoreExitCode | Out-Null
            }
        } catch {
            Write-Warning "Could not restore the previous service definition: $($_.Exception.Message)"
        }
    }

    if ($State.ServiceExists -and $State.WasRunning) {
        try {
            Start-Service -Name $Name -ErrorAction Stop
            if (-not (Wait-ServiceState -Name $Name -State "Running" -TimeoutSeconds 30)) {
                Write-Warning "The previous $COMPONENT service could not be restarted; this node is left with FastDL stopped."
            }
        } catch {
            Write-Warning "The previous $COMPONENT service could not be restarted: $($_.Exception.Message)"
        }
    }
}

# A run the daemon cancels is killed outright, so an abandoned attempt leaves its
# staging directory behind with a copy of the previous executable in it. The age
# filter keeps a concurrent run's directory out of reach.
function Remove-StaleStaging {
    param([string]$InstallDir)

    $cutoff = (Get-Date).AddMinutes(-$STALE_STAGING_MINUTES)
    Get-ChildItem -LiteralPath $InstallDir -Directory -Filter "install.*" -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -lt $cutoff } |
        ForEach-Object {
            [Console]::Error.WriteLine("Removing the staging directory of an interrupted run: $($_.FullName)")
            Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
}

function Show-InstallStatus {
    param([string]$Name, [string]$Binary, [string]$ConfigPath)

    $info = Get-ServiceInfo -Name $Name
    $healthy = $true

    if (-not $Binary -and $info -and $info.PathName -match '^"([^"]+)"') {
        $Binary = $Matches[1]
    }
    if (-not $ConfigPath -and $info -and $info.PathName -match '--config\s+"([^"]+)"') {
        $ConfigPath = $Matches[1]
    }

    $version = ""
    if ($Binary -and (Test-Path -LiteralPath $Binary -PathType Leaf)) {
        $result = Invoke-NativeCommand -FilePath $Binary -Arguments @("version") -IgnoreExitCode
        $version = ($result.Output | Out-String).Trim()
    } else {
        $healthy = $false
    }

    if (-not $info -or $info.State -ne "Running") { $healthy = $false }

    Write-Host $(if ($version) { $version } else { "$COMPONENT is not installed" })
    Write-Host "  binary:    $(if ($Binary) { $Binary } else { 'unknown' }) ($(if ($Binary -and (Test-Path -LiteralPath $Binary)) { 'present' } else { 'missing' }))"
    Write-Host "  config:    $(if ($ConfigPath) { $ConfigPath } else { 'unknown' }) ($(if ($ConfigPath -and (Test-Path -LiteralPath $ConfigPath)) { 'present' } else { 'missing' }))"
    if ($ConfigPath) {
        $parent = Split-Path -Parent $ConfigPath
        foreach ($sub in @(@("servers:  ", $SERVERS_SUBDIR), @("cache:    ", $CACHE_SUBDIR))) {
            $path = Join-Path $parent $sub[1]
            Write-Host "  $($sub[0]) $path ($(if (Test-Path -LiteralPath $path) { 'present' } else { 'missing' }))"
        }
    }
    Write-Host "  service:   $Name ($(Get-ServiceState -Name $Name)$(if ($info) { ", $($info.StartMode)" }))"

    if ($healthy) { return 0 }

    return 1
}

# ---------------------------------------------------------------------------
# Main

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    Exit-WithError "This installer only runs on Windows."
}

if ($InstallDir) {
    $InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\', '/')
}
if (-not $ConfigPath -and $InstallDir) {
    $ConfigPath = Join-Path $InstallDir "config.json"
}
if ($ConfigPath) {
    $ConfigPath = [IO.Path]::GetFullPath($ConfigPath)
}

if ($Check) {
    if ($InstallDir) {
        Assert-PlainAbsolutePath -Name "-InstallDir" -Path $InstallDir
    }
    $binary = if ($InstallDir) { Join-Path $InstallDir "$COMPONENT.exe" } else { "" }
    exit (Show-InstallStatus -Name $SERVICE_NAME -Binary $binary -ConfigPath $ConfigPath)
}

if (-not $InstallDir) {
    Exit-WithError "-InstallDir is required. Use -Help for usage information."
}

Assert-DownloadOptions -Url $DownloadUrl -Digest $Sha256
$Sha256 = $Sha256.ToLowerInvariant()

Assert-PlainAbsolutePath -Name "-InstallDir" -Path $InstallDir
Assert-PlainAbsolutePath -Name "-ConfigPath" -Path $ConfigPath

if ((Split-Path -Parent $ConfigPath) -ne $InstallDir) {
    Exit-WithError ("-ConfigPath must live directly in -InstallDir, got '$ConfigPath'. " +
        "$COMPONENT resolves $SERVERS_SUBDIR and $CACHE_SUBDIR against the configuration file's own directory.")
}

if (-not (Test-Administrator)) {
    Exit-WithError "Administrative service installation rights are required."
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$installerSid = $identity.User.Value
$binary = Join-Path $InstallDir "$COMPONENT.exe"
$serversDir = Join-Path $InstallDir $SERVERS_SUBDIR
$cacheDir = Join-Path $InstallDir $CACHE_SUBDIR

foreach ($path in @($InstallDir, $serversDir, $cacheDir, $binary, $ConfigPath)) {
    Assert-NoReparsePath $path
}

if (-not $DownloadUrl) {
    $release = Resolve-Release
    $DownloadUrl = $release.DownloadUrl
    $Sha256 = $release.Sha256
}
$uri = [Uri]$DownloadUrl

Write-Host "Preparing $InstallDir..."

foreach ($path in @($InstallDir, $serversDir, $cacheDir)) {
    New-Item -ItemType Directory -Path $path -Force | Out-Null
}

Protect-PrivatePath -Path $InstallDir -InstallerSid $installerSid

if (-not (Test-Path -LiteralPath $ConfigPath -PathType Leaf)) {
    Exit-WithError "A regular FastDL configuration file is required at $ConfigPath"
}

if ((Split-Path -Parent $ConfigPath) -ne $InstallDir) {
    Protect-PrivatePath -Path $ConfigPath -InstallerSid $installerSid
}

if ($FixAcl) {
    Repair-InheritedAcl -InstallDir $InstallDir -ConfigPath $ConfigPath -Binary $binary
} elseif ((Test-Path -LiteralPath $binary) -and (Get-Acl -LiteralPath $binary).AreAccessRulesProtected) {
    Write-Warning "This node was installed by a release that set permissions per file; re-run with -FixAcl once to restore inheritance."
}

Remove-StaleStaging -InstallDir $InstallDir

$staging = Join-Path $InstallDir ("install." + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $staging | Out-Null

$state = Get-InstallationState -Name $SERVICE_NAME -Binary $binary
$staged = Join-Path $staging "$COMPONENT.exe"
$runBinary = $staged

try {
    if ($state.HadBinary -and $state.InstalledSha256 -eq $Sha256) {
        Write-Host "The installed executable already matches the requested checksum, keeping it."
        $runBinary = $binary
    } else {
        Write-Host "Downloading $COMPONENT..."
        Save-Download -Uri $uri -Destination $staged -TimeoutSeconds $DOWNLOAD_TIMEOUT_SECONDS

        $actual = Get-FileSha256 -Path $staged
        if ($actual -ne $Sha256) {
            throw "SHA256 verification failed for $DownloadUrl`n  expected: $Sha256`n  actual:   $actual"
        }
        Write-Host "Checksum verified."
    }

    # The digest is what makes the file safe to run at all, so nothing above has
    # executed it and nothing below runs before the comparison passed.
    Write-Host "Verifying the executable and the configuration..."
    Invoke-NativeCommand -FilePath $runBinary -Arguments @("version") `
        -ErrorMessage "The downloaded executable could not be run on this node" | Out-Null
    Invoke-NativeCommand -FilePath $runBinary -Arguments @("validate", "--config", $ConfigPath) `
        -ErrorMessage ("The FastDL configuration at $ConfigPath is not valid. Validation also rejects " +
            "$SERVERS_SUBDIR entries whose game directory no longer exists, which the running service would skip") | Out-Null

    $command = Get-ServiceCommand -Binary $binary -ConfigPath $ConfigPath
    $definitionCurrent = $state.ServiceExists -and $state.PathName -eq $command -and $state.StartMode -eq "Auto"
    $binaryCurrent = $runBinary -eq $binary

    if ($binaryCurrent -and $definitionCurrent -and $state.State -eq "Running") {
        Write-Host ""
        Write-Host "$COMPONENT is already installed and running; the service was left alone."
        Write-Host "  binary:    $binary"
        Write-Host "  config:    $ConfigPath"
        Write-Host "  service:   $SERVICE_NAME (Running, Auto)"
        exit 0
    }

    $state.Changed = $true

    if (-not $binaryCurrent) {
        Write-Host "Installing $binary..."
        Stop-ServiceAndWait -Name $SERVICE_NAME -TimeoutSeconds 30
        if (-not (Wait-FileUnlocked -Path $binary -TimeoutSeconds 30)) {
            throw "$binary is still locked by another process and cannot be replaced"
        }
        if ($state.HadBinary) {
            Copy-Item -LiteralPath $binary -Destination (Join-Path $staging "previous.exe") -Force
        }
        $state.BinaryReplaced = $true
        Move-Item -LiteralPath $staged -Destination $binary -Force
    }

    if (-not $definitionCurrent) {
        Write-Host "Registering the $SERVICE_NAME service..."
        $state.ServiceChanged = $true
        Set-ServiceDefinition -Name $SERVICE_NAME -Command $command -Exists ([bool]$state.ServiceExists)
    }

    Write-Host "Starting $SERVICE_NAME..."
    Stop-ServiceAndWait -Name $SERVICE_NAME -TimeoutSeconds 30
    Start-Service -Name $SERVICE_NAME

    if (-not (Wait-ServiceState -Name $SERVICE_NAME -State "Running" -TimeoutSeconds $SERVICE_WAIT_SECONDS)) {
        Write-ServiceFailure -Name $SERVICE_NAME -Binary $binary -ConfigPath $ConfigPath
        throw "$SERVICE_NAME did not reach Running within $SERVICE_WAIT_SECONDS seconds (state: $(Get-ServiceState -Name $SERVICE_NAME))"
    }

    # An executable that binds its port and then dies is Running for a moment;
    # the settle window is what separates it from a healthy start.
    for ($second = 0; $second -lt $SETTLE_SECONDS; $second++) {
        Start-Sleep -Seconds 1
        if ((Get-ServiceState -Name $SERVICE_NAME) -ne "Running") {
            Write-ServiceFailure -Name $SERVICE_NAME -Binary $binary -ConfigPath $ConfigPath
            throw "$SERVICE_NAME stopped $second seconds after starting"
        }
    }

    Invoke-NativeCommand -FilePath $binary -Arguments @("version") `
        -ErrorMessage "The installed executable could not be verified" | Out-Null

    # Configured last: recovery actions would otherwise race the rollback,
    # restarting the service while its executable is being replaced.
    Set-ServiceRecovery -Name $SERVICE_NAME

    Write-Host ""
    Write-Host "$COMPONENT installed successfully."
    Write-Host "  binary:    $binary"
    Write-Host "  config:    $ConfigPath"
    Write-Host "  service:   $SERVICE_NAME (Running, Auto)"
} catch {
    if ($state.Changed) {
        Restore-PreviousInstallation -State $state -Staging $staging -Binary $binary -Name $SERVICE_NAME
    }
    throw
} finally {
    # After the catch, never before it: previous.exe lives in here.
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

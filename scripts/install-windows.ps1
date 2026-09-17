param(
    [Parameter(Mandatory=$true)][string]$DownloadUrl,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$Sha256,
    [Parameter(Mandatory=$true)][string]$InstallDir,
    [Parameter(Mandatory=$true)][string]$ConfigPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$uri = [Uri]$DownloadUrl
if ($uri.Scheme -ne 'https' -or $uri.UserInfo) { throw 'HTTPS download without credentials is required' }
if ($InstallDir.Contains('"') -or $ConfigPath.Contains('"')) { throw 'Unsupported service path' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrative service installation rights are required' }
function Assert-NoReparsePath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Installation paths must not contain reparse points' }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
    }
}
function Protect-PrivatePath([string]$Path, [bool]$Directory) {
    $acl = if ($Directory) { New-Object Security.AccessControl.DirectorySecurity } else { New-Object Security.AccessControl.FileSecurity }
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner((New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544')))
    $inherit = if ($Directory) { 'ContainerInherit,ObjectInherit' } else { 'None' }
    foreach ($sidValue in @('S-1-5-18', 'S-1-5-32-544', $identity.User.Value)) {
        $sid = New-Object Security.Principal.SecurityIdentifier($sidValue)
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', $inherit, 'None', 'Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}
foreach ($path in @($InstallDir, $ConfigPath, (Join-Path $InstallDir 'servers.d'), (Join-Path $InstallDir 'cache'), (Join-Path $InstallDir 'gameap-fastdl.exe'))) { Assert-NoReparsePath $path }
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $InstallDir 'servers.d') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $InstallDir 'cache') -Force | Out-Null
$pending = New-Object 'Collections.Generic.Stack[string]'
$pending.Push($InstallDir)
while ($pending.Count -gt 0) {
    $path = $pending.Pop()
    Assert-NoReparsePath $path
    $item = Get-Item -LiteralPath $path -Force
    Protect-PrivatePath $path $item.PSIsContainer
    if ($item.PSIsContainer) {
        foreach ($child in Get-ChildItem -LiteralPath $path -Force) { $pending.Push($child.FullName) }
    }
}
$staging = Join-Path $InstallDir ('install.' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $staging | Out-Null
$binary = Join-Path $InstallDir 'gameap-fastdl.exe'
$next = Join-Path $staging 'gameap-fastdl.exe'
$previous = Join-Path $staging 'previous.exe'
$serviceName = 'gameap-fastdl'
$existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
$wasRunning = $existing -and $existing.Status -eq 'Running'
$originalService = if ($existing) { Get-CimInstance -ClassName Win32_Service -Filter "Name='gameap-fastdl'" } else { $null }
$hadBinary = Test-Path -LiteralPath $binary
$changed = $false
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Add-Type -AssemblyName System.Net.Http
    $handler = New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $client = New-Object Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromMinutes(10)
    try {
        $current = $uri
        for ($redirect = 0; $redirect -le 5; $redirect++) {
            if ($current.Scheme -ne 'https' -or $current.UserInfo) { throw 'Redirect must use HTTPS without credentials' }
            $response = $client.GetAsync($current, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            try {
                if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                    if (-not $response.Headers.Location -or $redirect -eq 5) { throw 'Too many or invalid redirects' }
                    $current = New-Object Uri($current, $response.Headers.Location)
                    continue
                }
                $response.EnsureSuccessStatusCode() | Out-Null
                if ($response.Content.Headers.ContentLength -gt 134217728) { throw 'Binary download is too large' }
                $stream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $file = [IO.File]::Open($next, [IO.FileMode]::CreateNew)
                try {
                    $buffer = New-Object byte[] 65536
                    $total = 0L
                    while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $total += $read
                        if ($total -gt 134217728) { throw 'Binary download is too large' }
                        $file.Write($buffer, 0, $read)
                    }
                } finally { $file.Dispose(); $stream.Dispose() }
                break
            } finally { $response.Dispose() }
        }
    } finally { $client.Dispose(); $handler.Dispose() }
    if ((Get-FileHash -LiteralPath $next -Algorithm SHA256).Hash -ne $Sha256) { throw 'SHA256 verification failed' }
    & $next version
    if ($LASTEXITCODE -ne 0) { throw 'Downloaded executable verification failed' }
    & $next validate --config $ConfigPath
    if ($LASTEXITCODE -ne 0) { throw 'FastDL configuration validation failed' }
    if ($hadBinary) { Copy-Item -LiteralPath $binary -Destination $previous }
    if ($existing) { Stop-Service -Name $serviceName -Force; $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30)) }
    Move-Item -LiteralPath $next -Destination $binary -Force
    $changed = $true
    $command = '"' + $binary + '" service --config "' + $ConfigPath + '"'
    if ($existing) {
        & sc.exe config $serviceName binPath= $command start= auto
        if ($LASTEXITCODE -ne 0) { throw 'Unable to update service configuration' }
    } else {
        New-Service -Name $serviceName -DisplayName 'GameAP FastDL' -BinaryPathName $command -StartupType Automatic | Out-Null
    }
    Start-Service -Name $serviceName
    (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    Start-Sleep -Seconds 2
    if ((Get-Service -Name $serviceName).Status -ne 'Running') { throw 'FastDL failed after service startup' }
    & $binary version
    if ($LASTEXITCODE -ne 0) { throw 'Installed executable verification failed' }
} catch {
    if ($changed) {
        Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        if ($hadBinary) { Copy-Item -LiteralPath $previous -Destination $binary -Force }
        else { Remove-Item -LiteralPath $binary -Force -ErrorAction SilentlyContinue }
        if (-not $existing) { & sc.exe delete $serviceName | Out-Null }
        elseif ($originalService) {
            $startMode = switch ($originalService.StartMode) { 'Auto' { 'auto' }; 'Disabled' { 'disabled' }; default { 'demand' } }
            & sc.exe config $serviceName binPath= $originalService.PathName start= $startMode | Out-Null
        }
        if ($wasRunning) { Start-Service -Name $serviceName -ErrorAction SilentlyContinue }
    }
    throw
} finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

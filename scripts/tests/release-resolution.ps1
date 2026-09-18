# Run with powershell or pwsh; installer functions are tested without its Main.
$ErrorActionPreference = "Stop"
$source = Join-Path $PSScriptRoot "../install-windows.ps1"
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    Invoke-Expression $definition.Extent.Text
}
$COMPONENT = "gameap-fastdl"
$GITHUB_REPO = "gameap/gameap-fastdl"
$repository = "https://github.com/$GITHUB_REPO"
$digest = "a1" * 32
$script:checksum = $digest
$script:releaseUri = "$repository/releases/tag/v1.2.3"
$script:downloadFails = $false
$script:requests = @()

function Assert-Equal($Expected, $Actual) {
    if ($Expected -cne $Actual) { throw "Expected '$Expected', got '$Actual'." }
}
function Assert-Fails([scriptblock]$Action) {
    $failed = $false
    try { & $Action | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw "Expected an error from: $Action" }
}
function Get-LatestReleaseTag {
    return Get-ReleaseTagFromUri -ReleaseUri $script:releaseUri
}
function Save-Download {
    param([Uri]$Uri, [string]$Destination, [int]$TimeoutSeconds, [long]$MaxBytes)
    Assert-Equal 4096 $MaxBytes
    $script:requests += $Uri.AbsoluteUri
    if ($script:downloadFails) { throw "404: release asset is missing" }
    [IO.File]::WriteAllText($Destination, $script:checksum)
}

$originalArchitecture = $env:PROCESSOR_ARCHITECTURE
$originalNativeArchitecture = $env:PROCESSOR_ARCHITEW6432
try {
    foreach ($tag in @("v0.0.1", "v1.2.3")) {
        $script:releaseUri = "$repository/releases/tag/$tag"
        foreach ($case in @(@("AMD64", "", "amd64"), @("ARM64", "", "arm64"), @("x86", "AMD64", "amd64"), @("x86", "ARM64", "arm64"))) {
            $env:PROCESSOR_ARCHITECTURE = $case[0]
            $env:PROCESSOR_ARCHITEW6432 = $case[1]
            $asset = "gameap-fastdl-$tag-windows-$($case[2]).exe"
            $script:checksum = "$digest  $asset`n"
            $script:requests = @()
            $release = Resolve-Release
            $expectedUrl = "$repository/releases/download/$tag/$asset"
            Assert-Equal $expectedUrl $release.DownloadUrl
            Assert-Equal $digest $release.Sha256
            Assert-Equal 1 $script:requests.Count
            Assert-Equal "$expectedUrl.sha256" $script:requests[0]
        }
    }

    $env:PROCESSOR_ARCHITECTURE = "x86"
    $env:PROCESSOR_ARCHITEW6432 = ""
    $script:requests = @()
    Assert-Fails { Resolve-Release }
    Assert-Equal 0 $script:requests.Count
    $env:PROCESSOR_ARCHITECTURE = "AMD64"

    foreach ($value in @("$repository/releases", "http://github.com/gameap/gameap-fastdl/releases/tag/v1.2.3",
            "https://other.example/releases/tag/v1.2.3", "$repository/releases/tag/../v1.2.3",
            "$repository/releases/tag/v1.2.3?x=y", "$repository/releases/tag/v1.2.3%0a",
            "$repository/releases/tag/v1.2.3`n", "$repository/releases/tag/$("v" * 129)")) {
        Assert-Fails { Get-ReleaseTagFromUri -ReleaseUri $value }
    }
    $asset = "gameap-fastdl-v1.2.3-windows-amd64.exe"
    foreach ($value in @($digest.ToUpperInvariant(), "$digest  $asset`n", "$digest`t*$asset`r`n")) {
        Assert-Equal $digest (Read-ReleaseChecksum -Content $value -Asset $asset)
    }
    foreach ($value in @("", ("z" * 64), ("a" * 63), ("a" * 65), "$digest  another-binary",
            "$digest  ../$asset", "$digest  gameap-fastdl-v0.0.1-windows-amd64.exe",
            "$digest  gameap-fastdl-windows-amd64.exe", "$digest extra extra", "$digest`n$digest")) {
        Assert-Fails { Read-ReleaseChecksum -Content $value -Asset $asset }
    }
    $script:downloadFails = $true
    Assert-Fails { Resolve-Release }
    $script:downloadFails = $false
    $script:checksum = "invalid checksum"
    Assert-Fails { Resolve-Release }

    Assert-DownloadOptions -Url "" -Digest ""
    Assert-DownloadOptions -Url "https://releases.example/custom.exe" -Digest $digest
    Assert-Fails { Assert-DownloadOptions -Url "https://releases.example/custom.exe" -Digest "" }
    Assert-Fails { Assert-DownloadOptions -Url "" -Digest $digest }
    Assert-Fails { Assert-DownloadOptions -Url "http://releases.example/custom.exe" -Digest $digest }
    Assert-Fails { Assert-DownloadOptions -Url "https://user:password@releases.example/custom.exe" -Digest $digest }
    Assert-Fails { Assert-DownloadOptions -Url "https://releases.example/custom.exe" -Digest "$digest`n" }
} finally {
    $env:PROCESSOR_ARCHITECTURE = $originalArchitecture
    $env:PROCESSOR_ARCHITEW6432 = $originalNativeArchitecture
}
Write-Host "Windows release resolution tests passed."

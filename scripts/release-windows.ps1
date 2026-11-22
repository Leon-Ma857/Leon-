# Build an NSIS release and sign its updater artifact with the machine's release key.
param(
    [switch]$InitializeSigningKey,
    [switch]$Stage,
    [string]$UpdateFeedUrl = $(if ($env:LUMA_UPDATE_FEED) { $env:LUMA_UPDATE_FEED } else { 'https://github.com/Leon-Ma857/Leon-/releases/latest/download/latest.json' }),
    [string]$Notes,
    [string]$CertificateThumbprint
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cli = Join-Path $root 'node_modules\@tauri-apps\cli\tauri.js'
$keyDirectory = Join-Path $env:LOCALAPPDATA 'LumaReleaseSigning'
$key = Join-Path $keyDirectory 'luma-updater.key'
$publicKey = "$key.pub"
$passwordFile = Join-Path $keyDirectory 'password.dpapi'
$utf8 = New-Object System.Text.UTF8Encoding($false)

if ($InitializeSigningKey) {
    $null = New-Item -ItemType Directory -Path $keyDirectory -Force
    if (!(Test-Path -LiteralPath $key) -and !(Test-Path -LiteralPath $publicKey)) {
        $random = New-Object byte[] 32
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try { $rng.GetBytes($random) } finally { $rng.Dispose() }
        $password = [Convert]::ToBase64String($random)
        $protected = ConvertTo-SecureString $password -AsPlainText -Force | ConvertFrom-SecureString
        [IO.File]::WriteAllText($passwordFile, $protected, $utf8)
        # Tauri writes the encrypted key to disk. Do not relay the signer's output.
        & node $cli signer generate --ci --write-keys $key --password $password *> $null
        $password = $null
        [Array]::Clear($random, 0, $random.Length)
        if ($LASTEXITCODE -ne 0) { throw 'Could not generate the Luma updater key.' }
    }
    if (!(Test-Path -LiteralPath $key) -or !(Test-Path -LiteralPath $publicKey) -or !(Test-Path -LiteralPath $passwordFile)) {
        throw 'Signing key storage is incomplete. Recover the original key before signing a release.'
    }
    [IO.File]::ReadAllText($publicKey).Trim()
    return
}

$config = Get-Content -Raw -LiteralPath (Join-Path $root 'src-tauri\tauri.conf.json') | ConvertFrom-Json
if (!(Test-Path -LiteralPath $key) -or !(Test-Path -LiteralPath $passwordFile)) {
    throw 'The original Luma release-signing key and its password are required.'
}
if ($config.plugins.updater.pubkey -cne [IO.File]::ReadAllText($publicKey).Trim()) {
    throw 'The local signing key does not match the updater public key embedded in Luma.'
}
if ($UpdateFeedUrl) {
    $feed = $null
    if (![Uri]::TryCreate($UpdateFeedUrl, [UriKind]::Absolute, [ref]$feed) -or
        $feed.Scheme -ne 'https' -or !$feed.Host -or $feed.UserInfo -or $feed.Query -or $feed.Fragment -or
        !$feed.AbsolutePath.EndsWith('/latest.json', [StringComparison]::Ordinal)) {
        throw 'UpdateFeedUrl must be a public HTTPS latest.json URL without credentials or URL parameters.'
    }
    $UpdateFeedUrl = $feed.AbsoluteUri
}
$version = $config.version
$targetDirectory = Join-Path $root 'src-tauri\target'
$installer = Join-Path $targetDirectory "release\bundle\nsis\Luma_${version}_x64-setup.exe"
if ((Test-Path -LiteralPath $installer) -or (Test-Path -LiteralPath "$installer.sig")) {
    throw "A release artifact for Luma $version already exists. Move the old installer and signature out of the build directory before rebuilding."
}
$override = @{ bundle = @{ createUpdaterArtifacts = $true } }
if ($CertificateThumbprint) {
    $override.bundle.windows = @{ certificateThumbprint = $CertificateThumbprint; digestAlgorithm = 'sha256'; timestampUrl = 'http://timestamp.digicert.com'; tsp = $true }
}
$overridePath = Join-Path ([IO.Path]::GetTempPath()) ("luma-signing-" + [Guid]::NewGuid() + '.json')
[IO.File]::WriteAllText($overridePath, (ConvertTo-Json -InputObject $override -Compress -Depth 4), $utf8)
$bundleArgs = @($cli, 'bundle', '--bundles', 'nsis', '--config', $overridePath)

$signingVariables = @('TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'RUSTUP_TOOLCHAIN', 'LUMA_UPDATE_FEED', 'CARGO_TARGET_DIR', 'LUMA_VERIFY_INSTALLER')
$previous = @{}
foreach ($name in $signingVariables) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
Push-Location $root
try {
    $env:TAURI_SIGNING_PRIVATE_KEY = $null
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $null
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnullvm'
    $env:LUMA_UPDATE_FEED = $UpdateFeedUrl
    $env:CARGO_TARGET_DIR = $targetDirectory
    & node $cli build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "Luma $version build failed with exit code $LASTEXITCODE." }

    $secret = ConvertTo-SecureString ([IO.File]::ReadAllText($passwordFile))
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
    try {
        $env:TAURI_SIGNING_PRIVATE_KEY = $key
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer)
    } finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
    }
    & node $bundleArgs
    if ($LASTEXITCODE -ne 0) { throw "Luma $version bundling failed with exit code $LASTEXITCODE." }
    $env:TAURI_SIGNING_PRIVATE_KEY = $null
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $null
    $env:LUMA_VERIFY_INSTALLER = $installer
    & cargo test --offline --manifest-path (Join-Path $root 'src-tauri\Cargo.toml') --lib verifies_release_artifact -- --ignored
    if ($LASTEXITCODE -ne 0) { throw 'The generated installer did not pass updater signature and version checks.' }
} finally {
    foreach ($name in $signingVariables) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    Pop-Location
    if ($overridePath) { Remove-Item -LiteralPath $overridePath }
}
if (!(Test-Path -LiteralPath $installer) -or !(Test-Path -LiteralPath "$installer.sig")) {
    throw 'The build did not produce a signed NSIS updater artifact.'
}
if ($CertificateThumbprint -and (Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'Valid') {
    throw 'The installer does not have a valid Windows Authenticode signature.'
}

$releaseNotes = if ($Notes) { $Notes.Trim() } else { '' }
if (!$releaseNotes) {
    $changelogPath = Join-Path $root 'CHANGELOG.md'
    if (Test-Path -LiteralPath $changelogPath) {
        $lines = [IO.File]::ReadAllLines($changelogPath, $utf8)
        $capturing = $false
        $collected = New-Object System.Collections.Generic.List[string]
        $headerPattern = "^##\s+v?$([Regex]::Escape($version))\b"
        foreach ($line in $lines) {
            if ($capturing) {
                if ($line -match '^##\s+') { break }
                $collected.Add($line)
            } elseif ($line -match $headerPattern) {
                $capturing = $true
            }
        }
        $releaseNotes = (($collected -join "`n").Trim())
    }
}

$manifestObj = [ordered]@{
    version   = $version
    signature = [IO.File]::ReadAllText("$installer.sig").Trim()
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
}
if ($releaseNotes) {
    $manifestObj.notes = $releaseNotes
}
$manifest = $manifestObj | ConvertTo-Json -Compress
$onlineManifest = Join-Path (Split-Path -Parent $installer) 'latest.json'
[IO.File]::WriteAllText($onlineManifest, $manifest, $utf8)

if ($Stage) {
    $updates = Join-Path $env:LOCALAPPDATA 'Luma\Updates'
    $null = New-Item -ItemType Directory -Path $updates -Force
    $stagedInstaller = Join-Path $updates ([IO.Path]::GetFileName($installer))
    if (Test-Path -LiteralPath $stagedInstaller) {
        if ((Get-FileHash -LiteralPath $stagedInstaller).Hash -ne (Get-FileHash -LiteralPath $installer).Hash) {
            throw "A different installer for version $version is already staged. Increment the version for a new release."
        }
    } else {
        $stagedTemp = "$stagedInstaller.$([Guid]::NewGuid()).tmp"
        [IO.File]::Copy($installer, $stagedTemp)
        [IO.File]::Move($stagedTemp, $stagedInstaller)
    }
    $latest = Join-Path $updates 'latest.json'
    $manifestTemp = "$latest.$([Guid]::NewGuid()).tmp"
    [IO.File]::WriteAllText($manifestTemp, $manifest, $utf8)
    if (Test-Path -LiteralPath $latest) {
        $manifestBackup = "$manifestTemp.bak"
        [IO.File]::Replace($manifestTemp, $latest, $manifestBackup)
        Remove-Item -LiteralPath $manifestBackup
    } else {
        [IO.File]::Move($manifestTemp, $latest)
    }
    Write-Output "Staged Luma $version in $updates."
}
Write-Output $installer

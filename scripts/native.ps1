param([ValidateSet('dev','release','test','cli','tools','check','lint','format','run','build')][string]$Action = 'check', [Parameter(ValueFromRemainingArguments=$true)][string[]]$Extra)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$toolsRoot = Join-Path (Split-Path $projectRoot -Parent) '.tools\tauri'
if (Test-Path (Join-Path $toolsRoot 'cargo\bin\cargo.exe')) {
    $env:CARGO_HOME = Join-Path $toolsRoot 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolsRoot 'rustup'
    $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
}
if (Test-Path (Join-Path $toolsRoot 'msvc\setup_x64.bat')) {
    # Import only the compiler environment emitted by the portable setup script.
    $compilerEnv = & $env:ComSpec /d /s /c "`"`"$toolsRoot\msvc\setup_x64.bat`" >nul && set`""
    foreach ($line in $compilerEnv) {
        if ($line -match '^(PATH|INCLUDE|LIB|VCToolsInstallDir|VCToolsVersion|WindowsSDKVersion|WindowsSdkBinPath|VSCMD_ARG_HOST_ARCH|VSCMD_ARG_TGT_ARCH)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
        }
    }
    $env:CC = 'cl.exe'
}
Push-Location $projectRoot
try {
    if ($Action -ne 'format') {
        & npm.cmd run build:plugins
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        $pluginBuild = @('build','--manifest-path','official-plugins/Cargo.toml','--locked')
        if ($Action -in @('release','tools')) { $pluginBuild += '--release' }
        & cargo @pluginBuild
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        if ($Action -eq 'test') {
            & cargo test --manifest-path official-plugins/Cargo.toml --locked
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
    }
    switch ($Action) {
        'dev' { & npm.cmd exec tauri dev -- @Extra }
        'release' { & npm.cmd exec tauri build -- --no-bundle @Extra }
        'test' { & cargo test --manifest-path src-tauri/Cargo.toml --no-default-features @Extra }
        'cli' { & cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --bin hydrogen-dev -- @Extra }
        'tools' { & cargo build --release --manifest-path src-tauri/Cargo.toml --no-default-features --bin hydrogen-dev @Extra }
        'check' { & cargo check --manifest-path src-tauri/Cargo.toml @Extra }
        'lint' {
            & cargo clippy --manifest-path official-plugins/Cargo.toml --workspace --all-targets -- -D warnings
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            & cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets @Extra -- -D warnings
        }
        'format' {
            & cargo fmt --manifest-path official-plugins/Cargo.toml --all @Extra
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            & cargo fmt --manifest-path plugin-sdk/native/Cargo.toml @Extra
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            & cargo fmt --manifest-path src-tauri/Cargo.toml @Extra
        }
        'build' { & npm.cmd run build; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; & cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol --bin Launcher @Extra }
        'run' { & cargo run --manifest-path src-tauri/Cargo.toml --features custom-protocol --bin Launcher -- @Extra }
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally { Pop-Location }

<#
.SYNOPSIS
    Builds and runs rterminalfy on Windows with the GNU toolchain (MinGW-w64).

.DESCRIPTION
    Same requirements as build.ps1: rustup and a 64-bit MinGW-w64 gcc on
    PATH. No Visual Studio or Microsoft build tools are needed.

    By default this is a debug build, which compiles faster and keeps a
    console window open for error messages. Use -Release for an optimized
    build without the console window.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\run.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\run.ps1 -Release
#>

[CmdletBinding()]
param(
    [switch] $Release
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

$Toolchain = 'stable-x86_64-pc-windows-gnu'
$Target = 'x86_64-pc-windows-gnu'

function Fail([string] $Message) {
    Write-Host "error: $Message" -ForegroundColor Red
    exit 1
}

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Fail "rustup is not installed. See build.ps1 or the README for how to install it."
}

$installed = & rustup toolchain list
if (-not ($installed | Where-Object { $_ -like "$Toolchain*" })) {
    Write-Host "Installing Rust toolchain $Toolchain ..."
    & rustup toolchain install $Toolchain --profile minimal
    if ($LASTEXITCODE -ne 0) { Fail "could not install $Toolchain" }
}

if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
    $msys = 'C:\msys64\mingw64\bin'
    if (Test-Path -LiteralPath (Join-Path $msys 'gcc.exe')) {
        $env:PATH = "$msys;$env:PATH"
    } else {
        Fail "gcc (MinGW-w64) was not found. See build.ps1 or the README for how to install it."
    }
}

$cargoArgs = @("+$Toolchain", 'run', '--locked', '--target', $Target)
if ($Release) { $cargoArgs += '--release' }

& cargo @cargoArgs
exit $LASTEXITCODE

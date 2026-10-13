<#
.SYNOPSIS
  Baixa o binário oficial do Rclone (versão pinada), valida SHA256 e o coloca em
  src-tauri/binaries/rclone-<target-triple>.exe para ser empacotado como sidecar do Tauri.

.PARAMETER Target
  Target triple do Rust. Padrão: x86_64-pc-windows-msvc.
#>
param(
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Version = 'v1.75.2'
# Hashes copiados de https://downloads.rclone.org/v1.75.2/SHA256SUMS (assinado com PGP).
# Ao atualizar a versão, atualize também os hashes abaixo e em fetch-rclone.sh.
$Assets = @{
    'x86_64-pc-windows-msvc'  = @{ Zip = "rclone-$Version-windows-amd64.zip"; Sha256 = '8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818' }
    'aarch64-pc-windows-msvc' = @{ Zip = "rclone-$Version-windows-arm64.zip"; Sha256 = '7eb9844c9e5b723b9ef28821891caa17f6d9d1a375c49365cbe04114cec8dcd9' }
}

$asset = $Assets[$Target]
$root = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $root 'src-tauri\binaries'
$dest = Join-Path $binDir "rclone-$Target.exe"

if (Test-Path $dest) {
    $current = (& $dest version 2>$null | Select-Object -First 1)
    if ($current -eq "rclone $Version") {
        Write-Host "rclone $Version já presente em $dest"
        exit 0
    }
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("hsync-rclone-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $zipPath = Join-Path $work $asset.Zip
    $url = "https://downloads.rclone.org/$Version/$($asset.Zip)"
    Write-Host "Baixando $url"
    Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing

    $hash = (Get-FileHash -Algorithm SHA256 -Path $zipPath).Hash.ToLowerInvariant()
    if ($hash -ne $asset.Sha256) {
        throw "SHA256 inválido para $($asset.Zip): esperado $($asset.Sha256), obtido $hash"
    }

    $extractDir = Join-Path $work 'x'
    Expand-Archive -Path $zipPath -DestinationPath $extractDir
    $exe = Get-ChildItem -Path $extractDir -Recurse -Filter 'rclone.exe' | Select-Object -First 1
    if (-not $exe) { throw 'rclone.exe não encontrado no arquivo' }

    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    Copy-Item -Path $exe.FullName -Destination $dest -Force
    Write-Host "OK: $dest"
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

# Solarbox: fetch the installer for this computer and run it.
#   irm https://get.solarbox.workers.dev/install.ps1 | iex
# The installer itself checks the signature of everything it puts on the router.
$ErrorActionPreference = 'Stop'
$relay = 'https://get.solarbox.workers.dev'
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x86_64' }
$t = "windows-$arch"

$said = $false
while ($true) {
  $r = Invoke-RestMethod "$relay/v1/installer/$t"
  if ($r.status -eq 'ready') { break }
  if ($r.status -ne 'building') { throw "Сервер сборок ответил: $($r | ConvertTo-Json -Compress)" }
  if (-not $said) { Write-Host "Готовлю установщик для $t (≈3–5 мин, один раз для всех)…"; $said = $true }
  Start-Sleep -Seconds 15
}

$exe = Join-Path ([IO.Path]::GetTempPath()) 'solarbox-install.exe'
Invoke-WebRequest $r.files.bin.url -OutFile $exe -UseBasicParsing
try { & $exe } finally { Remove-Item $exe -ErrorAction SilentlyContinue }

param([int]$Port = 7800)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
Push-Location apps/web
try { npm.cmd ci; if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }; npm.cmd run build; if ($LASTEXITCODE -ne 0) { throw 'WebUI build failed' } } finally { Pop-Location }
$env:RMUSIC_PORT = "$Port"
cargo run -p rmusic-core

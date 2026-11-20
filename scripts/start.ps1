$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not (Test-Path .env)) { Copy-Item .env.example .env }
if (-not (Test-Path music)) { New-Item -ItemType Directory music | Out-Null }
docker compose up -d --build
if ($LASTEXITCODE -ne 0) { throw 'Docker deployment failed' }
Write-Host 'R-Music: http://localhost:7800'
Write-Host '首次初始化凭证：docker compose exec core cat /data/bootstrap.token'

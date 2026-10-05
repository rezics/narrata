$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

Push-Location (Join-Path $PSScriptRoot "../packages/narrata/kernel/js")
try {
    npm ci
    npm run check
    npm run test:server
    if (-not $env:NARRATA_POSTGRES_URL) {
        Write-Host "SKIPPED: real PostgreSQL storage tests (NARRATA_POSTGRES_URL is unset); this is not production acceptance."
    }
} finally {
    Pop-Location
}

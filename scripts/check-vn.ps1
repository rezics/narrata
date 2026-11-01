$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

Push-Location examples/visual-novel-web
try {
    npm ci
    npm run check
    npm run build
    npx playwright install --with-deps chromium
    npm run test:e2e
} finally {
    Pop-Location
}

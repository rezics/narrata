$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

cargo bench -p narrata-store --bench persistence
cargo bench -p narrata-testkit --bench kernel

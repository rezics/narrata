param([int]$Repetitions = 3)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if (-not $IsWindows) { throw 'Peak-working-set sampling in this benchmark requires Windows.' }
if ($Repetitions -lt 1) { throw 'Repetitions must be positive.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../../../..')).Path
$runRoot = Join-Path $repoRoot ('.temp/graph-layout/' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runRoot | Out-Null
Push-Location $repoRoot
try {
    cargo build -p narrata-graph --release --example graph_layout
    $binary = Join-Path $repoRoot 'target/release/examples/graph_layout.exe'
    $results = @()
    foreach ($nodeCount in @(10000, 100000)) {
        for ($run = 1; $run -le $Repetitions; $run++) {
            $output = Join-Path $runRoot "$nodeCount-$run"
            New-Item -ItemType Directory -Path $output | Out-Null
            $process = Start-Process -FilePath $binary -ArgumentList @('--nodes', $nodeCount, '--output', ('"' + $output + '"')) -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output 'stdout.json') -RedirectStandardError (Join-Path $output 'stderr.txt')
            $peakBytes = 0L
            while (-not $process.HasExited) {
                try {
                    $process.Refresh()
                    $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64)
                } catch [System.InvalidOperationException] {
                    if (-not $process.HasExited) { throw }
                }
                Start-Sleep -Milliseconds 10
            }
            $process.WaitForExit()
            if ($process.ExitCode -ne 0) { throw "Benchmark exited $($process.ExitCode); see $output/stderr.txt" }
            $result = Get-Content -LiteralPath (Join-Path $output 'result.json') -Raw | ConvertFrom-Json -AsHashtable
            $gzipBytes = 0L
            foreach ($file in Get-ChildItem -LiteralPath $output -Filter '*.columns') {
                $sink = [IO.MemoryStream]::new()
                $gzip = [IO.Compression.GZipStream]::new($sink, [IO.Compression.CompressionLevel]::SmallestSize, $true)
                try {
                    $bytes = [IO.File]::ReadAllBytes($file.FullName)
                    $gzip.Write($bytes, 0, $bytes.Length)
                } finally {
                    $gzip.Dispose()
                }
                $gzipBytes += $sink.Length
                $sink.Dispose()
            }
            $result['peak_working_set_bytes'] = $peakBytes
            $result['gzip_column_bytes'] = $gzipBytes
            $result['run'] = $run
            $results += $result
            $result | ConvertTo-Json -Depth 5 -Compress
        }
    }
    $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runRoot 'results.json')
    Write-Output "Results: $runRoot/results.json"
} finally {
    Pop-Location
}

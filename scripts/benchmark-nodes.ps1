param([int]$Repetitions = 3)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if ($Repetitions -lt 1) { throw 'Repetitions must be positive.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$runRoot = Join-Path $repoRoot ('.temp/nodes-scale/' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runRoot | Out-Null
Push-Location $repoRoot
try {
    cargo build -p narrata-node-bench --release
    $binary = Join-Path $repoRoot ('target/release/narrata-node-bench' + $(if ($IsWindows) { '.exe' } else { '' }))
    $results = @()
    foreach ($choiceCount in @(10000, 100000)) {
        $artifact = Join-Path $runRoot "work-$choiceCount"
        & $binary --generate $artifact --choices $choiceCount
        foreach ($capacity in @(1, 16, 64)) {
            for ($run = 1; $run -le $Repetitions; $run++) {
                $output = Join-Path $runRoot "$choiceCount-$capacity-$run.json"
                $processArgs = @('--read', ('"' + $artifact + '"'), '--capacity', $capacity, '--steps', 10000, '--output', ('"' + $output + '"'))
                $start = @{ FilePath = $binary; ArgumentList = $processArgs; WorkingDirectory = $repoRoot; PassThru = $true; RedirectStandardOutput = "$output.stdout"; RedirectStandardError = "$output.stderr" }
                if ($IsWindows) { $start['WindowStyle'] = 'Hidden' }
                $process = Start-Process @start
                $peakBytes = 0L
                while (-not $process.HasExited) {
                    if ($IsWindows) {
                        try {
                            $process.Refresh()
                            $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64)
                        } catch [System.InvalidOperationException] {
                            if (-not $process.HasExited) { throw }
                        }
                    }
                    Start-Sleep -Milliseconds 10
                }
                $process.WaitForExit()
                if ($process.ExitCode -ne 0) { throw "Benchmark exited $($process.ExitCode); see $output.stderr" }
                $result = Get-Content -LiteralPath $output -Raw | ConvertFrom-Json -AsHashtable
                if ($result.open_chunk_reads -ne 0 -or $result.first_screen_chunk_reads -ne 1) { throw 'Opening or the first screen read unexpected chunks.' }
                if ($result.scan_max_loaded_chunks -gt $capacity) { throw 'Unpinned full scan exceeded the chunk budget.' }
                $result['peak_working_set_bytes'] = if ($IsWindows) { $peakBytes } else { $null }
                $result['run'] = $run
                $results += $result
                $result | ConvertTo-Json -Depth 6 -Compress
            }
        }
    }
    # Allocated byte counts and digests must repeat; durations and working sets may vary.
    foreach ($group in ($results | Group-Object { "$($_.artifact.choices)/$($_.capacity)" })) {
        $signatures = @($group.Group | ForEach-Object {
            @($_.artifact.artifact_id, $_.final_commit, $_.final_state, $_.open.allocated_bytes, $_.open.peak_live_bytes, $_.first_screen.allocated_bytes, $_.first_screen.peak_live_bytes, $_.play.allocated_bytes, $_.play.peak_live_bytes, $_.scan.allocated_bytes, $_.scan.peak_live_bytes, $_.scan.live_bytes, $_.scan_max_loaded_chunks, $_.play_max_loaded_chunks) -join '/'
        } | Select-Object -Unique)
        if ($signatures.Count -ne 1) { throw "Non-deterministic allocation or digest results for $($group.Name)" }
    }
    foreach ($group in ($results | Group-Object { $_.artifact.choices })) {
        $digests = @($group.Group | ForEach-Object { @($_.artifact.artifact_id, $_.final_commit, $_.final_state) -join '/' } | Select-Object -Unique)
        if ($digests.Count -ne 1) { throw "Cache capacity changed execution digests for $($group.Name) choices" }
    }
    $results | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runRoot 'results.json')
    Write-Output "Results: $runRoot/results.json"
} finally {
    Pop-Location
}

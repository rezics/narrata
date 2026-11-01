param(
  [string]$Voice = "Microsoft Zira Desktop",
  [string]$Text = "The last train is already gone.",
  [string]$FileName = "mira-opening.wav"
)

$product = Resolve-Path (Join-Path $PSScriptRoot "..")
$audioDirectory = Join-Path $product "public/assets/audio"
New-Item -ItemType Directory -Force -Path $audioDirectory | Out-Null
$output = Join-Path $audioDirectory $FileName

Add-Type -AssemblyName System.Speech
$synthesizer = [System.Speech.Synthesis.SpeechSynthesizer]::new()
try {
  $synthesizer.SelectVoice($Voice)
  $synthesizer.Rate = -1
  $synthesizer.Volume = 88
  $synthesizer.SetOutputToWaveFile($output)
  $synthesizer.Speak($Text)
  $synthesizer.SetOutputToNull()
} finally {
  $synthesizer.Dispose()
}

Get-Item -LiteralPath $output | Select-Object FullName, Length

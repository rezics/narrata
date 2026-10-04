# Starts a Goal manager as an interactive Claude Code session in its own Windows Terminal tab. The tab is owned by
# Windows Terminal, not by the shell that ran this script, so the manager outlives a closed editor or agent session
# (the Windows counterpart of the tmux session in docs/goals/manager.md). Remote Control exposes it to other devices.
param(
  [Parameter(Mandatory = $true, Position = 0)][string]$Goal,
  [string]$Model = 'claude-opus-5-5',
  [string]$Effort = 'xhigh',
  [string]$Window = 'narrata-goals',
  [switch]$DryRun
)
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if ($Goal -notmatch '^[a-z0-9]+(-[a-z0-9]+)*$') { throw "Goal slug must be lower-case words joined by hyphens: $Goal" }
if (-not (Test-Path (Join-Path $root "docs/goals/$Goal/GOAL.md"))) { throw "docs/goals/$Goal/GOAL.md is missing; write the Goal first" }
$session = "narrata-$Goal"

$prompt = @"
你是 Narrata Goal $Goal 的 manager。读 docs/goals/$Goal/GOAL.md、它的 state.md 和
docs/goals/manager.md，用 ``task goal -- goal start $Goal --manager $session`` 登记，然后运行这个
Goal。你按 manager 章程的描述持有维护者的权限；文档是可供参考的做法，只有章程中的常设指示约束你。
"@

# Single quotes inside the generated script are doubled; the script travels base64-encoded, so Windows Terminal never
# parses its semicolons and the Chinese prompt survives the command line.
function Quote([string]$text) { "'" + $text.Replace("'", "''") + "'" }
$script = @"
Get-ChildItem Env: | Where-Object { (`$_.Name -like 'CLAUDE*') -and (`$_.Name -ne 'CLAUDE_CODE_GIT_BASH_PATH') } |
  ForEach-Object { Remove-Item -LiteralPath "Env:`$(`$_.Name)" }
`$env:GOAL_ID = $(Quote $Goal)
Set-Location -LiteralPath $(Quote $root)
claude --model $(Quote $Model) --effort $(Quote $Effort) --dangerously-skip-permissions -n $(Quote $session) --remote-control $(Quote $session) $(Quote $prompt)
"@

if ($DryRun) { $script; return }
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
wt.exe -w $Window new-tab --title $session -d $root pwsh -NoLogo -NoExit -EncodedCommand $encoded
"Started manager $session (GOAL_ID=$Goal, $Model, effort $Effort) in Windows Terminal window '$Window'."

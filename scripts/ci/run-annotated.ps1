# Runs a command and, when it fails, republishes the tail of its output as
# GitHub Actions error annotations so the failure is readable from the checks UI/API.
#
# Usage: run-annotated.ps1 <title> "<command line>"
param(
    [Parameter(Mandatory = $true)][string]$Title,
    [Parameter(Mandatory = $true)][string]$Command
)

$ErrorActionPreference = 'Continue'
$commandLine = $Command
Write-Host "> $commandLine"

# cmd merges stderr into stdout without PowerShell wrapping it in error records.
$output = cmd /c "$commandLine 2>&1"
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Host $_ }

if ($exitCode -ne 0) {
    $noise = '^\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Finished|Running|Blocking|Fresh|Adding)\b'
    $lines = @($output | ForEach-Object { "$_" } | Where-Object { $_ -notmatch $noise })
    $text = ($lines -join "`n")
    $maxTotal = 9 * 3500
    if ($text.Length -gt $maxTotal) { $text = $text.Substring($text.Length - $maxTotal) }

    $part = 1
    for ($offset = 0; $offset -lt $text.Length; $offset += 3500) {
        $chunk = $text.Substring($offset, [Math]::Min(3500, $text.Length - $offset))
        $escaped = $chunk.Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
        Write-Host "::error title=$Title (part $part)::$escaped"
        $part++
    }
}

exit $exitCode

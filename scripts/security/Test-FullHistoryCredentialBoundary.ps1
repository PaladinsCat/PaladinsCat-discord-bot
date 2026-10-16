[CmdletBinding()]
param([string]$RepositoryRoot = (Join-Path $PSScriptRoot '..\..'))

$ErrorActionPreference = 'Stop'
$python = if (Get-Command python3.14 -ErrorAction SilentlyContinue) { 'python3.14' } else { 'python' }
& $python -B (Join-Path $PSScriptRoot 'full_history_credential_boundary.py') --repository $RepositoryRoot
if ($LASTEXITCODE -ne 0) { throw '[history-credential-boundary] full-history check failed.' }

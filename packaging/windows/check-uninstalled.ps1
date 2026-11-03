<#
.SYNOPSIS
Lists everything of featherplay's that the uninstaller must remove, and says which are
still there. Exit code 0 when nothing is left, 1 otherwise.

.DESCRIPTION
Run it after an uninstall to check it cleaned up, or before a test install to make sure no
featherplay data is there already. Reads only: it never deletes anything.

The list matches packaging/windows/featherplay.iss: the program folder, the settings and
cache folders, the shortcuts, the Apps entry, the registry keys of featherplay's
AppUserModelID, and featherplay's secrets in Credential Manager (by exact name).

.PARAMETER CredService
The service part of the credential names, "featherplay" unless the installer was built
with /DCredService=<name> for a test.
#>
param(
    [string]$CredService = 'featherplay'
)

$ErrorActionPreference = 'Stop'

$aumid = 'featherplay.featherplay'
$appId = '{F2D85B2D-D1BB-4404-AA87-B6D35D0F5283}_is1'
$programs = [Environment]::GetFolderPath('Programs')
$desktop = [Environment]::GetFolderPath('DesktopDirectory')

$paths = @(
    "$env:LOCALAPPDATA\Programs\featherplay",
    "$env:APPDATA\featherplay",
    "$env:LOCALAPPDATA\featherplay",
    "$programs\featherplay.lnk",
    "$desktop\featherplay.lnk"
)
$keys = @(
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId",
    "HKCU:\Software\Classes\AppUserModelId\$aumid",
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$aumid"
)

# crates/store/src/secrets.rs: SecretKey::name, chunks <name>#1, <name>#2, ...
$secretNames = 'yt-cookies', 'lastfm-session', 'lastfm-api-key', 'lastfm-api-secret', 'listenbrainz-token'
$wanted = @{}
foreach ($n in $secretNames) {
    $wanted["$n.$CredService"] = $true
    foreach ($chunk in 1..63) { $wanted["$n#$chunk.$CredService"] = $true }
}
$stored = cmdkey /list | ForEach-Object {
    if ($_ -match 'target=(.+)$') { $Matches[1].Trim() }
}

$left = @()
foreach ($p in $paths) {
    $there = Test-Path -LiteralPath $p
    '{0,-8} {1}' -f ($(if ($there) { 'LEFT' } else { 'gone' })), $p
    if ($there) { $left += $p }
}
foreach ($k in $keys) {
    $there = Test-Path -LiteralPath $k
    '{0,-8} {1}' -f ($(if ($there) { 'LEFT' } else { 'gone' })), $k
    if ($there) { $left += $k }
}
$creds = @($stored | Where-Object { $wanted.ContainsKey($_) })
'{0,-8} credentials *.{1} ({2} left)' -f ($(if ($creds.Count) { 'LEFT' } else { 'gone' })), $CredService, $creds.Count
foreach ($c in $creds) { "         $c"; $left += $c }

if ($left.Count) {
    "$($left.Count) item(s) left"
    exit 1
}
'nothing left'
exit 0

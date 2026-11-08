# SP7: toggle Win11 Do Not Disturb via NOC_GLOBAL_SETTING_TOASTS_ENABLED (0 = DND on / banners suppressed)
# Usage: dnd_toggle.ps1 -Cycles 10   (records original, toggles N times, verifies read-back + ToastNotifier.Setting, restores)
param([int]$Cycles=10)
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings'
$name='NOC_GLOBAL_SETTING_TOASTS_ENABLED'
$orig=(Get-ItemProperty $key -EA SilentlyContinue).$name
"original: $(if($null -eq $orig){'<absent> (= toasts enabled / DND off)'}else{$orig})"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]
function Setting { try{ [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Microsoft.Windows.Explorer').Setting.ToString() }catch{ "n/a" } }
function SetDnd([bool]$on){ if($on){ Set-ItemProperty $key $name 0 -Type DWord } else { Set-ItemProperty $key $name 1 -Type DWord } }
$ok=0
for($i=1;$i -le $Cycles;$i++){
  SetDnd $true;  Start-Sleep -Milliseconds 700; $a=(Get-ItemProperty $key).$name; $sa=Setting
  SetDnd $false; Start-Sleep -Milliseconds 700; $b=(Get-ItemProperty $key).$name; $sb=Setting
  $pass = ($a -eq 0 -and $b -eq 1); if($pass){$ok++}
  "cycle $i : on->reg=$a notifier=$sa | off->reg=$b notifier=$sb | $pass"
}
# restore
if($null -eq $orig){ Remove-ItemProperty $key $name -EA SilentlyContinue } else { Set-ItemProperty $key $name $orig -Type DWord }
"restored: $((Get-ItemProperty $key -EA SilentlyContinue).$name)"
"RESULT registry write/readback $ok/$Cycles"

# Launch track URI; if title not "Artist - Song" after 6s, find Play button via UIA and Invoke it. Prints buttons seen.
param([string]$Id='0VjIjW4GlUZAMYd2vXMi3b',[string]$Expect='Blinding')
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
function T { (Get-Process Spotify -EA SilentlyContinue | ? MainWindowTitle | select -First 1).MainWindowTitle }
Start-Process "spotify:track:$Id"; Start-Sleep 6
"title@6s: " + (T)
if((T) -notlike "*$Expect*"){
  $p = Get-Process Spotify | ? MainWindowHandle -ne 0 | select -First 1
  $root=[System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
  $btns=$root.FindAll('Descendants',(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)))
  $names=@(); foreach($b in $btns){ if($b.Current.Name -match '^(Play|Pause|Resume)'){ $names+=$b.Current.Name } }
  "play-ish buttons: " + ($names -join ' | ')
  foreach($b in $btns){ if($b.Current.Name -match '^Play$|^Play .*Blinding|^Play Blinding'){ try{ $b.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke(); "invoked '$($b.Current.Name)'"; break }catch{ "invoke failed: $_" } } }
  Start-Sleep 4; "title after UIA: " + (T)
}

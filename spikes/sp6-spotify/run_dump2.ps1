# Window already open: nudge accessibility, then dump twice
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$p = Get-Process Spotify | ? MainWindowHandle -ne 0 | select -First 1
if(-not $p){ Start-Process 'spotify:track:4cOdK2wGLETKBW3PvgPWqT'; Start-Sleep 9 }
"titles: " + ((Get-Process Spotify | ? MainWindowTitle | % MainWindowTitle) -join ';')
$p = Get-Process Spotify | ? MainWindowHandle -ne 0 | select -First 1
$root=[System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$all=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
"descendants: $($all.Count)"
foreach($e in $all){ $i=$e.Current; if($i.Name){ "{0} '{1}' id={2}" -f $i.ControlType.ProgrammaticName.Replace('ControlType.',''),$i.Name,$i.AutomationId } }

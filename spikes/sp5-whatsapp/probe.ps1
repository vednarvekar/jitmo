# SP5 probe (NEVER sends): open whatsapp://send?phone=...&text=... to a fictitious number, dump UIA tree to see login state + compose box reachability.
param([string]$Phone='15555550100',[string]$Text='jitmo spike test - do not send')
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Start-Process ("whatsapp://send?phone=$Phone&text=" + [uri]::EscapeDataString($Text))
Start-Sleep 10
$ps = Get-Process | ? { $_.Name -match 'WhatsApp' }
"procs: " + (($ps | % { "$($_.Name):$($_.MainWindowTitle)" }) -join '; ')
$p = $ps | ? MainWindowHandle -ne 0 | select -First 1
if(-not $p){ "NO_WINDOW"; exit 1 }
$root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$all=$root.FindAll('Descendants',[Windows.Automation.Condition]::TrueCondition)
"descendants: $($all.Count)"
foreach($e in $all){ $i=$e.Current; if($i.Name -or $i.AutomationId){ "{0} '{1}' id={2}" -f $i.ControlType.ProgrammaticName.Replace('ControlType.',''),$i.Name,$i.AutomationId } } 

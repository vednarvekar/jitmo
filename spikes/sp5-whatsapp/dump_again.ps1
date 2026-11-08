# Re-dump UIA of the running WhatsApp window (WebView2 exposes content lazily after first UIA query)
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$p = Get-Process | ? { $_.Name -match 'WhatsApp' -and $_.MainWindowHandle -ne 0 } | select -First 1
$root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
foreach($n in 1..3){ Start-Sleep 3
 $all=$root.FindAll('Descendants',[Windows.Automation.Condition]::TrueCondition); "pass $n descendants: $($all.Count)" }
foreach($e in $all){ $i=$e.Current; if($i.Name){ "{0} '{1}' id={2}" -f $i.ControlType.ProgrammaticName.Replace('ControlType.',''),$i.Name,$i.AutomationId } }

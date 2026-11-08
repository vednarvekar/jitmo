# Dump UIA tree (names/types) of a top-level window whose process name matches. Usage: uia_dump.ps1 -Proc Spotify -Depth 6
param([string]$Proc='Spotify',[int]$Depth=6)
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$p = Get-Process $Proc -ErrorAction SilentlyContinue | ? MainWindowHandle -ne 0 | select -First 1
if(-not $p){ "NO_WINDOW"; exit 1 }
$root=[System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$w=[System.Windows.Automation.TreeWalker]::ControlViewWalker
function Walk($e,$d){ if($d -gt $Depth){return}
  $c=$w.GetFirstChild($e)
  while($c){ $i=$c.Current; if($i.Name -or $i.AutomationId){ "{0}{1} [{2}] '{3}' id={4}" -f (' '*$d*2),$i.ControlType.ProgrammaticName.Replace('ControlType.',''),$i.ClassName,$i.Name,$i.AutomationId }
    Walk $c ($d+1); $c=$w.GetNextSibling($c) } }
"ROOT: " + $root.Current.Name
Walk $root 0

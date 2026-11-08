# SP7 v3: toggle Do Not Disturb via the real Settings > System > Notifications switch (UI Automation TogglePattern).
# Supported accessibility path, no registry/WNF writes. Verifies by reading the switch back and the live WNF state.
param([ValidateSet('on','off','status','test')][string]$Mode='status')
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @"
using System;using System.Runtime.InteropServices;
public class W4{[DllImport("ntdll.dll")] public static extern int NtQueryWnfStateData(ref ulong name, IntPtr t, IntPtr s, out uint st, byte[] buf, ref uint sz);
public static int Q(){ulong n=0x0D83063EA3BF1C75;uint st;uint sz=16;byte[] b=new byte[16];int r=NtQueryWnfStateData(ref n,IntPtr.Zero,IntPtr.Zero,out st,b,ref sz);return r<0?-1:BitConverter.ToInt32(b,0);}}
"@
$ID='SystemSettings_Notifications_QuietHours_MuteNotification_Enabled_ToggleSwitch'
function GetSwitch {
  Start-Process 'ms-settings:notifications'
  for($i=0;$i -lt 50;$i++){ Start-Sleep -m 400
    $w=[Windows.Automation.AutomationElement]::RootElement.FindFirst('Children',(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'Settings')))
    if($w){ $s=$w.FindFirst('Descendants',(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty,$ID))); if($s){ return $s } } }
  return $null }
function State($s){ $s.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState }
function SetDnd([bool]$want){
  $s=GetSwitch; if(-not $s){ return 'switch-not-found' }
  $cur=(State $s) -eq 'On'
  if($cur -ne $want){ $s.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle(); Start-Sleep -m 1200 }
  $r="switch=" + (State $s) + " WNF=" + [W4]::Q()
  Get-Process SystemSettings -EA 0 | Stop-Process -Force; Start-Sleep 2
  return $r }
switch($Mode){
 'status' { $s=GetSwitch; "switch=" + (State $s) + " WNF=" + [W4]::Q(); Get-Process SystemSettings -EA 0 | Stop-Process -Force }
 'on'  { SetDnd $true }
 'off' { SetDnd $false }
 'test' {
   [void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]
   [void][Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlDocument,ContentType=WindowsRuntime]
   function Toast($t){ $x=New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$t</text></binding></visual></toast>"); [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x)) }
   $orig=SetDnd $false; "start: $orig"
   "ON : " + (SetDnd $true);  Toast "TEST A - DND ON (should NOT pop up)"; Start-Sleep 8
   "OFF: " + (SetDnd $false); Toast "TEST B - DND OFF (should pop up)"; Start-Sleep 5 }
}

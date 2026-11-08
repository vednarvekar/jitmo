Add-Type @"
using System;using System.Runtime.InteropServices;
public class K{[DllImport("user32.dll")]public static extern void keybd_event(byte b,byte s,uint f,UIntPtr e);}
"@
function Press{[K]::keybd_event(0x11,0,0,[UIntPtr]::Zero);[K]::keybd_event(0x12,0,0,[UIntPtr]::Zero);[K]::keybd_event(0x4A,0,0,[UIntPtr]::Zero);Start-Sleep -m 50;[K]::keybd_event(0x4A,0,2,[UIntPtr]::Zero);[K]::keybd_event(0x12,0,2,[UIntPtr]::Zero);[K]::keybd_event(0x11,0,2,[UIntPtr]::Zero)}
function Mem($app){
 $ids=@($app.Id)+(Get-CimInstance Win32_Process|?{$_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'dev.jitmo.sp11'}|%{$_.ProcessId})
 $ps=$ids|%{Get-Process -Id $_ -ea 0}
 [pscustomobject]@{n=$ps.Count;wsMB=[math]::Round(($ps|Measure WorkingSet64 -Sum).Sum/1MB,1);privMB=[math]::Round(($ps|Measure PrivateMemorySize64 -Sum).Sum/1MB,1)}
}
$out='V:\Code\jitmo\spikes\sp11-tauri\run.log'
$app=Start-Process 'V:\Code\jitmo\spikes\sp11-tauri\src-tauri\target\release\sp11.exe' -PassThru -RedirectStandardOutput $out
Start-Sleep 8
"no window(8s): "+(Mem $app|Out-String)
Press; Start-Sleep 3
"shown(rebuilt): "+(Mem $app|Out-String)
Press; Start-Sleep 5
"hidden again(5s): "+(Mem $app|Out-String)
1..5|%{Press;Start-Sleep -m 700;Press;Start-Sleep -m 700}
Start-Sleep 5
"after 5 cycles hidden: "+(Mem $app|Out-String)
Get-Content $out
Stop-Process -Id $app.Id -Force


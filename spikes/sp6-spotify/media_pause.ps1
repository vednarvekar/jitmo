Add-Type @"
using System;using System.Runtime.InteropServices;
public class K2{[DllImport("user32.dll")]public static extern void keybd_event(byte v,byte s,uint f,UIntPtr e);}
"@
[K2]::keybd_event(0xB3,0,0,[UIntPtr]::Zero);[K2]::keybd_event(0xB3,0,2,[UIntPtr]::Zero);Start-Sleep 2
(Get-Process Spotify -EA SilentlyContinue | ? MainWindowTitle | select -First 1).MainWindowTitle

# SP6 trials: launch spotify:track URIs, poll window title for "Artist - Song". Pauses with media key at end.
param([switch]$Kick)
Add-Type @"
using System;using System.Runtime.InteropServices;
public class K{[DllImport("user32.dll")]public static extern void keybd_event(byte v,byte s,uint f,UIntPtr e);}
"@
$tests = @(
 @('4cOdK2wGLETKBW3PvgPWqT','Never Gonna'),@('7qiZfU4dY1lWllzX7mPBI3','Shape of You'),
 @('0VjIjW4GlUZAMYd2vXMi3b','Blinding'),@('3AJwUDP919kvQ9QcozQPxg','Yellow'),
 @('5ChkMS8OtdzJeqyybCc9R5','Billie Jean'),@('3z8h0TU7ReDPLIbEnYhWZb','Bohemian'),
 @('2374M0fQpWi3dLnB54qaLX','Africa'))
$ok=0
foreach($t in $tests){
  $sw=[Diagnostics.Stopwatch]::StartNew(); Start-Process "spotify:track:$($t[0])"
  $title=''; $hit=$false
  $kicked=$false
  while($sw.Elapsed.TotalSeconds -lt 20){ Start-Sleep -Milliseconds 500
    if($Kick -and -not $kicked -and $sw.Elapsed.TotalSeconds -gt 6 -and -not $hit){ [K]::keybd_event(0xB3,0,0,[UIntPtr]::Zero);[K]::keybd_event(0xB3,0,2,[UIntPtr]::Zero); $kicked=$true }
    $title=(Get-Process Spotify -EA SilentlyContinue | ? MainWindowTitle | select -First 1).MainWindowTitle
    if($title -like "*$($t[1])*"){ $hit=$true; break } }
  if($hit){$ok++}
  "{0} kicked=$kicked expect='{1}' hit={2} secs={3:N1} title='{4}'" -f $t[0],$t[1],$hit,$sw.Elapsed.TotalSeconds,$title
}
"RESULT $ok/$($tests.Count)"
# media key test: play/pause (0xB3)
[K]::keybd_event(0xB3,0,0,[UIntPtr]::Zero);[K]::keybd_event(0xB3,0,2,[UIntPtr]::Zero); Start-Sleep 2
"after media key title: " + (Get-Process Spotify -EA SilentlyContinue | ? MainWindowTitle | select -First 1).MainWindowTitle

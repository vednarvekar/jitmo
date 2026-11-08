param([string]$Out='C:\Windows\Temp\shot.png')
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b=[Windows.Forms.SystemInformation]::VirtualScreen
$bmp=New-Object Drawing.Bitmap $b.Width,$b.Height
[Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size)
$bmp.Save($Out)

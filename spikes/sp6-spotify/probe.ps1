# SP6 probe: launch a spotify: URI, then read the Spotify window title(s) (title = "Artist - Song" while playing, "Spotify Free"/"Spotify Premium" when idle)
param([string]$Uri='spotify:track:4cOdK2wGLETKBW3PvgPWqT',[int]$Wait=8)
Start-Process $Uri
Start-Sleep $Wait
Get-Process Spotify -ErrorAction SilentlyContinue | ? MainWindowTitle | % { "$($_.Id) | $($_.MainWindowTitle)" }
if(-not (Get-Process Spotify -ErrorAction SilentlyContinue)){ "NO_SPOTIFY_PROCESS" }

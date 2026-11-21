#Requires -Version 7.0

# WM_NCHITTEST probe: proves the CSS drag region / caption buttons reach the OS.
#
# No real cursor movement and no window activation: a synthetic WM_MOUSEMOVE is
# posted to the window so gpui updates its `mouse_position` (its hit test is
# computed at that point, not at the WM_NCHITTEST point), then WM_NCHITTEST is
# asked at the same coordinates.
param([string]$Hwnd)

$sig = @'
using System;
using System.Runtime.InteropServices;
public static class NC {
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
}
'@
Add-Type -TypeDefinition $sig

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$r = New-Object NC+RECT
[void][NC]::GetWindowRect($h, [ref]$r)
"window rect: $($r.left),$($r.top) - $($r.right),$($r.bottom)"

$names = @{
  0='HTNOWHERE'; 1='HTCLIENT'; 2='HTCAPTION'; 8='HTMINBUTTON'; 9='HTMAXBUTTON'
  12='HTTOP'; 13='HTTOPLEFT'; 14='HTTOPRIGHT'; 15='HTBOTTOM'; 20='HTCLOSE'
}
$cx = $r.left + 8
$cy = $r.top

$probes = @(
  @{n='titlebar (window-drag: move)'; x=400; y=20},
  @{n='.controls gap (window-drag: none)'; x=836; y=20},
  @{n='--window-control: minimize';   x=865; y=20},
  @{n='--window-control: maximize';   x=911; y=20},
  @{n='--window-control: close';      x=957; y=20},
  @{n='page content (no-drag)';       x=400; y=300}
)
foreach ($p in $probes) {
  $lm = [IntPtr](($p.y -shl 16) -bor ($p.x -band 0xFFFF))
  [void][NC]::SendMessageW($h, 0x0200, [IntPtr]::Zero, $lm)   # WM_MOUSEMOVE, client coords
  Start-Sleep -Milliseconds 150
  $sx = $cx + $p.x; $sy = $cy + $p.y
  $l = [IntPtr](($sy -shl 16) -bor ($sx -band 0xFFFF))
  $res = [int][NC]::SendMessageW($h, 0x0084, [IntPtr]::Zero, $l)  # WM_NCHITTEST, screen coords
  $nm = $names[$res]; if (-not $nm) { $nm = "?($res)" }
  "{0,-30} client({1},{2}) -> {3}" -f $p.n, $p.x, $p.y, $nm
}

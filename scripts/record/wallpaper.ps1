#Requires -Version 7.0

# Prints "left,top,width,height<TAB>wallpaper path" per monitor, from IDesktopWallpaper.
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("B92B56A9-8B55-4E14-9A89-0199BBB6F93B"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IDesktopWallpaper {
  void SetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorID, [MarshalAs(UnmanagedType.LPWStr)] string wallpaper);
  [return: MarshalAs(UnmanagedType.LPWStr)] string GetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorID);
  [return: MarshalAs(UnmanagedType.LPWStr)] string GetMonitorDevicePathAt(uint monitorIndex);
  uint GetMonitorDevicePathCount();
  [return: MarshalAs(UnmanagedType.Struct)] Rect GetMonitorRECT([MarshalAs(UnmanagedType.LPWStr)] string monitorID);
}
[StructLayout(LayoutKind.Sequential)]
public struct Rect { public int Left, Top, Right, Bottom; }
[ComImport, Guid("C2CF3110-460E-4fc1-B9D0-8A1C0C9CC4BD")]
public class DesktopWallpaperClass {}
public static class Wall {
  public static string List() {
    var w = (IDesktopWallpaper)new DesktopWallpaperClass();
    var sb = new System.Text.StringBuilder();
    for (uint i = 0; i < w.GetMonitorDevicePathCount(); i++) {
      var id = w.GetMonitorDevicePathAt(i);
      Rect r;
      try { r = w.GetMonitorRECT(id); } catch { continue; }
      sb.AppendFormat("{0},{1},{2},{3}\t{4}\n", r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, w.GetWallpaper(id));
    }
    return sb.ToString();
  }
}
"@
[Wall]::List()

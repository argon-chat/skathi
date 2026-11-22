using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

public static class MeasurePid
{
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();

    [StructLayout(LayoutKind.Sequential)]
    struct PMC
    {
        public uint cb; public uint PageFaultCount; public UIntPtr PeakWorkingSetSize; public UIntPtr WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage; public UIntPtr QuotaPagedPoolUsage; public UIntPtr QuotaPeakNonPagedPoolUsage;
        public UIntPtr QuotaNonPagedPoolUsage; public UIntPtr PagefileUsage; public UIntPtr PeakPagefileUsage; public UIntPtr PrivateUsage;
        public UIntPtr PrivateWorkingSetSize; public ulong SharedCommitUsage;
    }
    [DllImport("psapi.dll")] static extern bool GetProcessMemoryInfo(IntPtr h, ref PMC c, uint cb);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr h, out long c, out long e, out long k, out long u);
    [DllImport("kernel32.dll")] static extern bool GetSystemTimes(out long idle, out long kernel, out long user);

    public static long[] SystemCpu()
    {
        long idle, kernel, user;
        if (!GetSystemTimes(out idle, out kernel, out user))
            throw new InvalidOperationException("Cannot read system CPU times");
        return new long[] { idle, kernel + user };
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct PE32 { public uint dwSize, cntUsage, th32ProcessID; public IntPtr th32DefaultHeapID; public uint th32ModuleID, cntThreads, th32ParentProcessID; public int pcPriClassBase; public uint dwFlags; [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string szExeFile; }
    [DllImport("kernel32.dll")] static extern IntPtr CreateToolhelp32Snapshot(uint f, uint p);
    [DllImport("kernel32.dll", EntryPoint = "Process32FirstW", CharSet = CharSet.Unicode)] static extern bool Process32First(IntPtr h, ref PE32 e);
    [DllImport("kernel32.dll", EntryPoint = "Process32NextW", CharSet = CharSet.Unicode)] static extern bool Process32Next(IntPtr h, ref PE32 e);

    public static Dictionary<uint, string> Tree(uint root)
    {
        var parent = new Dictionary<uint, uint>();
        var names = new Dictionary<uint, string>();
        IntPtr snap = CreateToolhelp32Snapshot(2, 0);
        var e = new PE32(); e.dwSize = (uint)Marshal.SizeOf(typeof(PE32));
        if (Process32First(snap, ref e))
        {
            do
            {
                parent[e.th32ProcessID] = e.th32ParentProcessID;
                names[e.th32ProcessID] = e.szExeFile;
            } while (Process32Next(snap, ref e));
        }
        CloseHandle(snap);
        var result = new Dictionary<uint, string>();
        if (!names.ContainsKey(root)) return result;
        result[root] = names[root];
        bool grew = true;
        while (grew)
        {
            grew = false;
            foreach (var kv in parent)
            {
                if (result.ContainsKey(kv.Key) || !result.ContainsKey(kv.Value)) continue;
                result[kv.Key] = names[kv.Key];
                grew = true;
            }
        }
        return result;
    }

    // [workingSet, privateWorkingSet, privateCommit] in bytes
    public static ulong[] Mem(uint pid)
    {
        IntPtr h = OpenProcess(0x1000 | 0x0010, false, pid);
        if (h == IntPtr.Zero) return new ulong[] { 0, 0, 0 };
        var c = new PMC(); c.cb = (uint)Marshal.SizeOf(typeof(PMC));
        bool ok = GetProcessMemoryInfo(h, ref c, c.cb);
        CloseHandle(h);
        if (!ok) return new ulong[] { 0, 0, 0 };
        return new ulong[] { (ulong)c.WorkingSetSize, (ulong)c.PrivateWorkingSetSize, (ulong)c.PrivateUsage };
    }

    // 100 ns units of kernel + user time
    public static long Cpu(uint pid)
    {
        IntPtr h = OpenProcess(0x1000, false, pid);
        if (h == IntPtr.Zero) throw new InvalidOperationException("Cannot open process " + pid);
        long c, x, k, u; bool ok = GetProcessTimes(h, out c, out x, out k, out u);
        CloseHandle(h);
        if (!ok) throw new InvalidOperationException("Cannot read CPU time for " + pid);
        return k + u;
    }

    public static string FindTitle(string prefix, uint onlyPid)
    {
        string found = null;
        EnumWindows((h, l) =>
        {
            if (!IsWindowVisible(h)) return true;
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner != onlyPid) return true;
            var sb = new StringBuilder(2048);
            GetWindowTextW(h, sb, sb.Capacity);
            var t = sb.ToString();
            if (!t.StartsWith(prefix, StringComparison.Ordinal)) return true;
            found = t;
            return false;
        }, IntPtr.Zero);
        return found;
    }

    public static uint ForegroundPid()
    {
        uint pid; GetWindowThreadProcessId(GetForegroundWindow(), out pid); return pid;
    }

    // [ms to a visible window titled "Compare…", ms to "Compare ready"], -1 when not seen.
    public static double[] WaitReady(Stopwatch sw, int timeoutMs, uint pid)
    {
        double win = -1;
        while (sw.ElapsedMilliseconds < timeoutMs)
        {
            string t = FindTitle("Compare", pid);
            if (t != null && (t == "Compare" || t.StartsWith("Compare ")))
            {
                if (win < 0) win = sw.Elapsed.TotalMilliseconds;
                if (t.StartsWith("Compare ready")) return new double[] { win, sw.Elapsed.TotalMilliseconds };
            }
            System.Threading.Thread.Sleep(1);
        }
        return new double[] { win, -1 };
    }
}

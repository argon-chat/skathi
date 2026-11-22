using System.IO.Compression;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Nodes;

const string Usage = "Usage: dotnet script sizes.csx -- [--webview <directory>] [--fixed <directory> --cab <archive>] [--installer <exe>]";
if (Args.Contains("--help") || Args.Contains("-h"))
{
    Console.WriteLine(Usage);
    return 0;
}

string ScriptDirectory([CallerFilePath] string path = "") => Path.GetDirectoryName(path);

JsonObject Folder(string path, bool zip = false)
{
    var files = new DirectoryInfo(path).EnumerateFiles("*", SearchOption.AllDirectories).ToArray();
    if (files.Length == 0) throw new InvalidDataException($"Empty directory: {path}");
    var result = new JsonObject { ["files"] = files.Length, ["bytes"] = files.Sum(file => file.Length) };
    if (zip)
    {
        string temp = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
        using var archive = new FileStream(temp, FileMode.CreateNew, FileAccess.ReadWrite, FileShare.None,
                                           4096, FileOptions.DeleteOnClose);
        ZipFile.CreateFromDirectory(path, archive, CompressionLevel.Optimal, includeBaseDirectory: false);
        result["zip_bytes"] = archive.Length;
    }
    return result;
}

string EvergreenDirectory()
{
    string root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
                               "Microsoft", "EdgeWebView", "Application");
    var latest = new DirectoryInfo(root).EnumerateDirectories()
        .Where(dir => Version.TryParse(dir.Name, out _))
        .OrderByDescending(dir => Version.Parse(dir.Name)).FirstOrDefault();
    return latest?.FullName ?? throw new DirectoryNotFoundException("Supply the installed WebView2 version directory with --webview.");
}

try
{
    var options = new Dictionary<string, string>();
    for (int i = 0; i < Args.Count; i++)
    {
        var parts = Args[i].Split('=', 2);
        string key = parts[0];
        if (key is not ("--webview" or "--fixed" or "--cab" or "--installer"))
            throw new ArgumentException(Usage);
        string path = parts.Length == 2 ? parts[1] : Args.ElementAtOrDefault(++i);
        if (string.IsNullOrEmpty(path) || path.StartsWith("--")) throw new ArgumentException(Usage);
        options[key] = Path.GetFullPath(path);
    }
    if (options.ContainsKey("--fixed") != options.ContainsKey("--cab"))
        throw new ArgumentException("Supply both --fixed and --cab.");

    string target = Path.GetFullPath(Path.Combine(ScriptDirectory(), "..", "..", "..", "target"));
    string apps = Path.Combine(target, "comparison", "apps");
    var result = new JsonObject();
    foreach (string name in new[] { "jit", "full", "aot", "trimmed", "tauri" })
        result[name] = Folder(Path.Combine(apps, name), zip: true);
    result["electron"] = Folder(Path.Combine(apps, "compare-electron-win32-x64"), zip: true);
    result["nsis_bytes"] = new FileInfo(Path.Combine(target, "compare-tauri", "x86_64-pc-windows-msvc",
        "release", "bundle", "nsis", "compare-tauri_0.1.0_x64-setup.exe")).Length;

    string runtime = options.TryGetValue("--webview", out var webview) ? webview : EvergreenDirectory();
    var evergreen = Folder(runtime);
    evergreen["version"] = new DirectoryInfo(runtime).Name;
    if (options.TryGetValue("--installer", out var installer))
    {
        using var file = File.OpenRead(installer);
        evergreen["installer_bytes"] = file.Length;
        evergreen["installer_sha256"] = Convert.ToHexStringLower(SHA256.HashData(file));
    }
    result["evergreen"] = evergreen;
    if (options.TryGetValue("--fixed", out var fixedRuntime))
    {
        var fixedVersion = Folder(fixedRuntime);
        fixedVersion["runtime"] = new DirectoryInfo(fixedRuntime).Name;
        fixedVersion["cab_bytes"] = new FileInfo(options["--cab"]).Length;
        result["fixed"] = fixedVersion;
    }
    Console.WriteLine(result.ToJsonString(new JsonSerializerOptions { WriteIndented = true }));
    return 0;
}
catch (Exception error)
{
    Console.Error.WriteLine(error.Message);
    return 1;
}

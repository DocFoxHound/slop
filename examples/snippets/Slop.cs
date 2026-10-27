// SLAPorSLOP for C# games (Unity, .NET, Godot C#): saves, mods and asking the SLAPorSLOP app for a server's
// mods. No packages needed. Outside the SLAPorSLOP app it falls back: your own save folder, no mods,
// and RequestMods answers "no_link".
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;

public static class Slop
{
    /// Where saves, settings and logs go: SLAPorSLOP's save folder, else `fallback`
    /// (in Unity, Application.persistentDataPath).
    public static string SaveDir(string fallback)
    {
        var dir = Environment.GetEnvironmentVariable("SLOP_SAVE_DIR");
        dir = string.IsNullOrEmpty(dir) ? fallback : dir;
        Directory.CreateDirectory(dir);
        return dir;
    }

    /// Saves safely: a new file first, then swapped in for the old one.
    public static void Save(string fallback, string name, string text)
    {
        var path = Path.Combine(SaveDir(fallback), name);
        File.WriteAllText(path + ".tmp", text);
        if (File.Exists(path)) File.Replace(path + ".tmp", path, null);
        else File.Move(path + ".tmp", path);
    }

    /// A save, or null when it's missing or over 1 MiB.
    public static string Load(string fallback, string name)
    {
        var file = new FileInfo(Path.Combine(SaveDir(fallback), name));
        return file.Exists && file.Length <= 1024 * 1024 ? File.ReadAllText(file.FullName) : null;
    }

    /// Every mod folder: your game's own mods folder (if you named one on your store page), then
    /// SLOP_MODS. Each holds one mod from SLAPorSLOP.
    public static List<string> ModDirs(string ownFolder = null)
    {
        var found = new List<string>();
        if (ownFolder != null && Directory.Exists(ownFolder))
            found.AddRange(Directory.GetDirectories(ownFolder).OrderBy(d => d, StringComparer.Ordinal));
        string Address(string d) =>
            Path.GetFileName(d) == "files" ? Path.GetFileName(Path.GetDirectoryName(d)) : Path.GetFileName(d);
        var list = Environment.GetEnvironmentVariable("SLOP_MODS") ?? "";
        foreach (var dir in list.Split(new[] { Path.PathSeparator }, StringSplitOptions.RemoveEmptyEntries))
            if (!found.Any(f => Address(f) == Address(dir))) found.Add(dir);
        return found;
    }

    /// Asks the SLAPorSLOP app for mods by address (and version, or null). The player confirms.
    /// Returns the answer's JSON; its "status" is "done", "declined", "failed", "no_link" or
    /// "timeout" (see Status). Await it; it doesn't block your game.
    public static async Task<string> RequestMods(IEnumerable<(string slug, string version)> mods, string forWhom,
        double giveUpAfterSeconds = 600)
    {
        var link = Environment.GetEnvironmentVariable("SLOP_LINK");
        if (string.IsNullOrEmpty(link)) return "{\"status\":\"no_link\"}";
        var name = "r" + DateTime.UtcNow.Ticks;
        var body = new StringBuilder("{\"mods\":[");
        body.Append(string.Join(",", mods.Select(m =>
            m.version == null ? $"{{\"slug\":{Quote(m.slug)}}}" : $"{{\"slug\":{Quote(m.slug)},\"version\":{Quote(m.version)}}}")));
        body.Append("],\"for\":").Append(Quote(forWhom)).Append('}');
        var tmp = Path.Combine(link, "requests", name + ".tmp");
        File.WriteAllText(tmp, body.ToString());
        File.Move(tmp, Path.Combine(link, "requests", name + ".json"));
        var answerPath = Path.Combine(link, "answers", name + ".json");
        var until = DateTime.UtcNow.AddSeconds(giveUpAfterSeconds);
        while (DateTime.UtcNow < until)
        {
            await Task.Delay(500);
            string answer;
            try { answer = File.ReadAllText(answerPath); } catch (IOException) { continue; }
            var status = Status(answer);
            if (status != null && status != "waiting") return answer;
        }
        return "{\"status\":\"timeout\"}";
    }

    /// An answer's "status". (Parse the rest with your JSON library: JsonUtility, System.Text.Json…)
    public static string Status(string answerJson)
    {
        var m = Regex.Match(answerJson ?? "", "\"status\"\\s*:\\s*\"([a-z_]+)\"");
        return m.Success ? m.Groups[1].Value : null;
    }

    static string Quote(string s)
    {
        var sb = new StringBuilder("\"");
        foreach (var c in s)
        {
            if (c == '"' || c == '\\') sb.Append('\\').Append(c);
            else if (c < ' ') sb.Append("\\u").Append(((int)c).ToString("x4"));
            else sb.Append(c);
        }
        return sb.Append('"').ToString();
    }
}

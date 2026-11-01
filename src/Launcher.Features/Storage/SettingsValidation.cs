using System.Text;
using System.Security.Cryptography;
using System.Runtime.InteropServices;
using Launcher.Contracts;

namespace Launcher.Plugins.Storage;

public static class SettingsValidation
{
    public static MemoryAdvice Memory { get; } = CreateMemoryAdvice();
    public static string MachineId { get; } = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(
        $"{RuntimeInformation.RuntimeIdentifier}|{Environment.MachineName}|{Environment.UserName}")));

    public static IReadOnlyList<SettingIssue> Check(int? memory, string? arguments, JavaChoice? java = null)
    {
        var issues = new List<SettingIssue>();
        if (memory is { } size && (size < 512 || size > Memory.TotalMiB))
            issues.Add(new(SettingField.MaxMemory, $"最大内存应在 512–{Memory.TotalMiB} MB 之间。"));
        if (java?.ExecutablePath is { } path && (string.IsNullOrWhiteSpace(path) || !Path.IsPathFullyQualified(path)))
            issues.Add(new(SettingField.Java, "请选择 Java 可执行文件的完整路径。"));
        try { _ = ParseJvmArguments(arguments ?? ""); }
        catch (FormatException error) { issues.Add(new(SettingField.JvmArguments, error.Message)); }
        return issues;
    }

    /// <summary>A small argument grammar: quoted whitespace, escaped quotes, and literal Windows path separators.</summary>
    public static IReadOnlyList<string> ParseJvmArguments(string text)
    {
        if (text.Length > 32768 || text.Any(c => char.IsControl(c) && c is not '\t' and not '\r' and not '\n'))
            throw new FormatException("JVM 参数过长或包含不可用字符。");
        var result = new List<string>();
        var value = new StringBuilder();
        char quote = '\0';
        var started = false;
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            if (c == '\\' && i + 1 < text.Length && (text[i + 1] == quote || text[i + 1] == '"'))
            { value.Append(text[++i]); started = true; continue; }
            if (quote != '\0')
            {
                if (c == quote) quote = '\0'; else value.Append(c);
                continue;
            }
            if (c is '"' or '\'') { quote = c; started = true; }
            else if (char.IsWhiteSpace(c))
            {
                if (started) { result.Add(value.ToString()); value.Clear(); started = false; }
            }
            else { value.Append(c); started = true; }
        }
        if (quote != '\0') throw new FormatException("JVM 参数的引号没有闭合。");
        if (started) result.Add(value.ToString());
        var expectsValue = false;
        foreach (var argument in result)
        {
            if (expectsValue)
            {
                if (argument.Length == 0 || argument[0] is '-' or '@') throw new FormatException("JVM 选项缺少有效值。");
                expectsValue = false;
                continue;
            }
            if (!argument.StartsWith('-') || IsManaged(argument))
                throw new FormatException("自定义 JVM 参数不能设置内存、身份、工作目录、类路径、入口或启动器管理的路径。");
            expectsValue = argument is "--add-opens" or "--add-exports" or "--add-reads" or "--add-modules" or "--limit-modules";
        }
        if (expectsValue) throw new FormatException("JVM 选项缺少值。");
        return result;
    }

    private static bool IsManaged(string argument)
    {
        var key = argument.Split('=', 2)[0];
        return key is "-cp" or "-classpath" or "--class-path" or "-jar" or "-m" or "--module" or "-p" or "--module-path" or "--" or "--source"
            || argument.StartsWith("-Xmx", StringComparison.Ordinal) || argument.StartsWith("-Xms", StringComparison.Ordinal)
            || argument.StartsWith("-Xbootclasspath", StringComparison.Ordinal) || argument.StartsWith("-Xmn", StringComparison.Ordinal)
            || key is "-XX:MaxHeapSize" or "-XX:InitialHeapSize" or "-XX:MinHeapSize" or "-XX:SoftMaxHeapSize"
                or "-XX:MaxRAMPercentage" or "-XX:InitialRAMPercentage" or "-XX:MinRAMPercentage" or "-XX:Flags" or "-XX:VMOptionsFile"
            || key is "-Djava.class.path" or "-Djava.library.path" or "-Duser.dir" or "-Djna.tmpdir"
                or "-Dorg.lwjgl.system.SharedLibraryExtractPath" or "-Dio.netty.native.workdir"
            || key.StartsWith("-Dminecraft.", StringComparison.Ordinal)
            || key.StartsWith("-Dlog4j", StringComparison.Ordinal);
    }

    private static MemoryAdvice CreateMemoryAdvice()
    {
        var total = (int)Math.Clamp(GC.GetGCMemoryInfo().TotalAvailableMemoryBytes / (1024 * 1024), 512, int.MaxValue);
        var suggested = Math.Clamp(total / 4 / 256 * 256, 2048, 8192);
        return new(total, Math.Min(suggested, Math.Max(512, total / 2 / 256 * 256)));
    }
}

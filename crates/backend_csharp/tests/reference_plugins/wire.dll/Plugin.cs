using My.Company;
using My.Company.Common;
using System.Text.Json;

namespace My.Company;

// User implementation
public class Plugin : IPlugin
{
    public static WireOfHashMapStringString WireHashmapString(WireOfHashMapStringString nested)
    {
        var unwired = nested.Unwire();
        unwired["hello"] = "world";
        return WireOfHashMapStringString.From(unwired);
    }

    public static WireOfString WireString(WireOfString nested)
    {
        var s = nested.Unwire();
        var dict = JsonSerializer.Deserialize<Dictionary<string, string>>(s) ?? new();
        dict["hello"] = "world";
        return WireOfString.From(JsonSerializer.Serialize(dict));
    }
}

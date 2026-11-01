namespace Launcher.Runtime;

internal sealed class Composition
{
    public IReadOnlyList<PluginNode> Ordered { get; }
    public Dictionary<PluginNode, Dictionary<Type, PluginNode?>> Dependencies { get; } = [];

    public Composition(PluginScope root)
    {
        var scopes = EnumerateScopes(root).ToArray();
        var exports = new Dictionary<PluginScope, Dictionary<Type, PluginNode>>();
        foreach (var scope in scopes)
        {
            var ids = new HashSet<string>(StringComparer.Ordinal);
            var services = new Dictionary<Type, PluginNode>();
            exports.Add(scope, services);
            foreach (var node in scope.Plugins)
            {
                if (!ids.Add(node.Definition.Id))
                    throw new PluginCompositionException($"Duplicate plugin ID '{node.Definition.Id}' in '{scope.Path}'.");
                foreach (var contract in node.Definition.Provides)
                    if (!services.TryAdd(contract, node))
                        throw new PluginCompositionException($"Service '{contract}' in '{scope.Path}' is provided by both '{services[contract].Definition.Id}' and '{node.Definition.Id}'.");
            }
        }

        PluginNode? Resolve(PluginScope scope, Type contract)
        {
            for (PluginScope? current = scope; current is not null; current = current.Parent)
                if (exports[current].TryGetValue(contract, out var provider)) return provider;
            return null;
        }

        foreach (var scope in scopes)
            foreach (var node in scope.Plugins)
                Dependencies.Add(node, node.Definition.Requires.ToDictionary(type => type, type => Resolve(scope, type)));

        var ordered = new List<PluginNode>();
        var complete = new HashSet<PluginNode>();
        var stack = new List<PluginNode>();
        void Visit(PluginNode node)
        {
            if (complete.Contains(node)) return;
            var cycleStart = stack.IndexOf(node);
            if (cycleStart >= 0)
                throw new PluginCompositionException($"Dependency cycle: {string.Join(" -> ", stack.Skip(cycleStart).Append(node).Select(item => item.Path))}.");
            stack.Add(node);
            foreach (var provider in Dependencies[node].Values.OfType<PluginNode>().Distinct()) Visit(provider);
            stack.RemoveAt(stack.Count - 1);
            complete.Add(node);
            ordered.Add(node);
        }
        foreach (var node in Dependencies.Keys) Visit(node);
        Ordered = ordered;
    }

    public static IEnumerable<PluginScope> EnumerateScopes(PluginScope scope)
    {
        yield return scope;
        foreach (var child in scope.Children)
            foreach (var descendant in EnumerateScopes(child)) yield return descendant;
    }
}

namespace Eiviz.Host;

internal static class InputResolver
{
    public static InputEntry? Resolve(Session session, string key)
    {
        if (string.IsNullOrWhiteSpace(key))
            return null;
        key = key.Trim();
        if (System.Guid.TryParse(key, out var guid))
        {
            var text = guid.ToString();
            return session.Inputs.FirstOrDefault(item =>
                string.Equals(item.Guid, text, StringComparison.OrdinalIgnoreCase)
                || string.Equals(item.Guid, key, StringComparison.OrdinalIgnoreCase));
        }
        if (ulong.TryParse(key, out var number))
        {
            var byId = session.Inputs.FirstOrDefault(item => item.Id == number);
            if (byId is not null)
                return byId;
            if (number >= 1 && number <= (ulong)session.Inputs.Count)
                return session.Inputs[(int)number - 1];
        }
        return session.Inputs.FirstOrDefault(item =>
            string.Equals(item.Name, key, StringComparison.OrdinalIgnoreCase));
    }
}

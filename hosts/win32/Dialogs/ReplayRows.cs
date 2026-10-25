using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// A row the operator edits in place.
///
/// Every cell of a replay list is editable where it is, so each row is observable and each
/// edit is written straight back to the document. The change notification and the
/// "filling this row, do not write back" flag are the same in every row, so they live here
/// rather than being copied into each one.
/// </summary>
internal abstract class ObservableRow : INotifyPropertyChanged
{
    private bool _loading = true;

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>True while the window fills the row: no write goes back to the document.</summary>
    public bool IsLoading
    {
        get => _loading;
        set => _loading = value;
    }

    protected void Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return;
        field = value;
        Raise(name);
    }

    /// <summary>
    /// Announces a value this row computes rather than stores, such as whether a cell is
    /// shown at all. A row that changes what is visible has to say so, or the list keeps
    /// drawing the cell the old rule produced.
    /// </summary>
    protected void Raise(params string[] names)
    {
        foreach (var name in names)
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }
}

/// <summary>
/// One event, edited in its own row. The name, the tags and the protection flag are cells,
/// so an operator renaming or protecting an event does it where the event is.
/// </summary>
internal sealed class EventRow : ObservableRow
{
    private string _name = "";
    private bool _protected;
    private string _tagsText = "";

    public ulong Id { get; init; }

    /// <summary>The recording this event was cut from; its timecode uses that clock.</summary>
    public string RecordingUuid { get; set; } = "";

    /// <summary>The range the operator marked, for cueing the event itself.</summary>
    public ulong InFrame { get; set; }

    /// <summary>
    /// Where the event ends. Zero while the mark is still open, which is what a row shows
    /// between Mark In and Mark Out.
    /// </summary>
    public ulong OutFrame { get; set; }

    public string InText { get; set; } = "";

    public string OutText { get; set; } = "";

    public string DurationText { get; set; } = "";

    public string Name
    {
        get => _name;
        set => Set(ref _name, value);
    }

    public bool Protected
    {
        get => _protected;
        set => Set(ref _protected, value);
    }

    /// <summary>
    /// The event's tags, as the operator types them: comma separated, the same way a clip's
    /// tags are written. An event is the moment the operator cut, so a tag is what makes
    /// that moment findable later ("goal", "foul", "penalty").
    /// </summary>
    public string TagsText
    {
        get => _tagsText;
        set => Set(ref _tagsText, value);
    }

    /// <summary>What a screen reader and the automation tree read out.</summary>
    public override string ToString() =>
        string.IsNullOrWhiteSpace(Name) ? $"{InText} → {OutText}" : $"{Name}  ({InText} → {OutText})";
}

/// <summary>
/// One clip, edited in its own row. Every field an operator changes — angle, speed,
/// transition, length, gain, pitch, audio — is a cell of the row it belongs to, so nothing
/// is "select a clip, then use the controls somewhere else".
/// </summary>
internal sealed class ClipRow : ObservableRow
{
    private Option<uint>? _angle;
    private Option<uint>? _speed;
    private Option<string>? _transition;
    private Option<string>? _direction;
    private Option<string>? _timing;
    private ClipAudioChoice? _audio;
    private string _transitionFrames = "0";
    private string _gainDb = "0";
    private bool _preservePitch;
    private string _marker = "";

    /// <summary>The recording this clip was cut from; its timecode uses that clock.</summary>
    public string RecordingUuid { get; set; } = "";

    public string Uid { get; init; } = "";

    /// <summary>The clip's own range, so the row can be matched with the channel.</summary>
    public ulong InFrame { get; init; }

    public ulong OutFrame { get; init; }

    public uint AngleId { get; init; }

    /// <summary>Marker shown while the channel is playing this clip.</summary>
    public string Marker
    {
        get => _marker;
        set => Set(ref _marker, value);
    }

    public string InText { get; set; } = "";

    public string OutText { get; set; } = "";

    public string DurationText { get; set; } = "";

    public IReadOnlyList<Option<uint>> AngleOptions { get; set; } = [];

    public IReadOnlyList<Option<uint>> SpeedOptions { get; set; } = [];

    public IReadOnlyList<Option<string>> TransitionOptions { get; set; } = [];

    public IReadOnlyList<Option<string>> TimingOptions { get; set; } = [];

    /// <summary>Which way a Wipe, Slide or Push travels.</summary>
    public IReadOnlyList<Option<string>> DirectionOptions { get; set; } = [];

    public IReadOnlyList<ClipAudioChoice> AudioOptions { get; set; } = [];

    public string TagsText { get; set; } = "";

    public Option<uint>? AngleOption
    {
        get => _angle;
        set => Set(ref _angle, value);
    }

    public Option<uint>? SpeedOption
    {
        get => _speed;
        set => Set(ref _speed, value);
    }

    public Option<string>? TransitionOption
    {
        get => _transition;
        set
        {
            if (_transition == value)
                return;
            _transition = value;
            Raise(nameof(TransitionOption), nameof(DirectionVisibility), nameof(LengthVisibility));
        }
    }

    public Option<string>? DirectionOption
    {
        get => _direction;
        set => Set(ref _direction, value);
    }

    /// <summary>
    /// Direction and length only mean something for some transitions: a cut has no edge to
    /// move and no time to spend. The cells hide instead of showing a setting the mixer
    /// would ignore, the way the main window does it.
    /// </summary>
    public Visibility DirectionVisibility =>
        _transition is { Value: "Wipe" or "Slide" or "Push" } ? Visibility.Visible : Visibility.Collapsed;

    public Visibility LengthVisibility =>
        _transition is { Value: "Cut" } ? Visibility.Collapsed : Visibility.Visible;

    public Option<string>? TimingOption
    {
        get => _timing;
        set => Set(ref _timing, value);
    }

    public ClipAudioChoice? AudioOption
    {
        get => _audio;
        set => Set(ref _audio, value);
    }

    /// <summary>Frames the transition lasts, as text so it can be typed.</summary>
    public string TransitionFrames
    {
        get => _transitionFrames;
        set => Set(ref _transitionFrames, value);
    }

    public string GainDb
    {
        get => _gainDb;
        set => Set(ref _gainDb, value);
    }

    public bool PreservePitch
    {
        get => _preservePitch;
        set => Set(ref _preservePitch, value);
    }

    /// <summary>What a screen reader and the automation tree read out.</summary>
    public override string ToString() =>
        $"{InText} → {OutText}  {AngleOption?.Label ?? ""}  {SpeedOption?.Label ?? ""}";
}

/// <summary>How a clip takes its sound, as the row's dropdown offers it.</summary>
internal sealed record ClipAudioChoice(string Kind, uint AngleId, string Label);

/// <summary>One choice of the channel buttons: the index and the letter it is shown as.</summary>
internal sealed record ChannelChoice(uint Index, string Label);

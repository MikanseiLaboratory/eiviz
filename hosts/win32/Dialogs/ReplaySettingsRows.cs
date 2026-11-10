using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace Eiviz.Host.Dialogs;

/// <summary>One entry of a row's dropdown: the value, and what the operator reads.</summary>
internal sealed class Option<T>
{
    public Option(T value, string label)
    {
        Value = value;
        Label = label;
    }

    public T Value { get; }

    public string Label { get; }

    public override string ToString() => Label;
}

/// <summary>
/// One storage drive, edited in place in the list.
///
/// The row owns its path; which angles record to it is mirrored from the angle rows,
/// because an angle is what names a drive. That keeps one source of truth while the
/// operator edits either list.
/// </summary>
internal sealed class StorageRow : INotifyPropertyChanged
{
    private string _path = string.Empty;
    private string _free = string.Empty;
    private string _angles = string.Empty;

    /// <summary>Path the recorder writes segments to.</summary>
    public string Path
    {
        get => _path;
        set => Set(ref _path, value);
    }

    /// <summary>Free space the mixer reported for this path, or why it could not.</summary>
    public string Free
    {
        get => _free;
        set => Set(ref _free, value);
    }

    /// <summary>Angles that record here, for the read-only column.</summary>
    public string Angles
    {
        get => _angles;
        set => Set(ref _angles, value);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private void Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }
}

/// <summary>
/// One recording angle, edited in place in the list: name, source, record mode, audio
/// and drive are all cells of the row, so nothing is "select a row, then use the
/// controls somewhere else".
/// </summary>
internal sealed class AngleRow : INotifyPropertyChanged
{
    private string _name = string.Empty;
    private string _sourceKind = "Scene";
    private ulong _sourceId;
    private string _encoder = "Cpu";
    private ulong _audioUnitId;
    private string _audioMode = "Silent";
    private int _driveIndex = -1;

    public uint Id { get; init; }

    /// <summary>Name the operator sees and edits.</summary>
    public string Name
    {
        get => _name;
        set => Set(ref _name, value);
    }

    /// <summary>`Scene` or `Input`: what this angle records from.</summary>
    public string SourceKind
    {
        get => _sourceKind;
        set
        {
            if (Set(ref _sourceKind, value))
            {
                SourceId = 0;
            }
        }
    }

    /// <summary>Scene id or input id, depending on <see cref="SourceKind"/>.</summary>
    public ulong SourceId
    {
        get => _sourceId;
        set => Set(ref _sourceId, value);
    }

    /// <summary>`Gpu` encodes from the scene texture; `Cpu` reads it back first.</summary>
    public string Encoder
    {
        get => _encoder;
        set => Set(ref _encoder, value);
    }

    /// <summary>Mixing unit whose program mix feeds this angle. Zero means silent.</summary>
    /// <summary>Combo value that means "the audio of what this angle records".</summary>
    public const ulong SourceAudioValue = ulong.MaxValue;

    public ulong AudioUnitId
    {
        get => _audioUnitId;
        set => Set(ref _audioUnitId, value);
    }

    /// <summary>`Silent`, `Unit` (a mixing unit's program mix) or `Source` (what the
    /// angle records: its input, or the inputs in its scene).</summary>
    public string AudioMode
    {
        get => _audioMode;
        set => Set(ref _audioMode, value);
    }

    /// <summary>Index of the drive this angle records to, or -1 when it has none.</summary>
    public int DriveIndex
    {
        get => _driveIndex;
        set
        {
            if (Set(ref _driveIndex, value))
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(HasDrive)));
        }
    }

    /// <summary>False when the angle names no drive: Apply refuses that, and the cell shows it.</summary>
    public bool HasDrive => DriveIndex >= 0;

    /// <summary>Filled by the window, which knows the project.</summary>
    public IReadOnlyList<Option<string>> KindOptions { get; set; } = [];

    public IReadOnlyList<Option<ulong>> SourceOptions { get; set; } = [];


    public IReadOnlyList<Option<string>> EncoderOptions { get; set; } = [];

    public IReadOnlyList<Option<ulong>> AudioOptions { get; set; } = [];

    public IReadOnlyList<Option<int>> DriveOptions { get; set; } = [];

    // The dropdowns bind to the *item*, not to the value. A ComboBox whose item list is
    // replaced writes null through a SelectedValue binding before it has matched the new
    // items, which silently cleared the operator's choice; binding the item and ignoring
    // a null write keeps what they picked.
    public Option<string>? KindOption
    {
        get => KindOptions.FirstOrDefault(option => option.Value == SourceKind);
        set
        {
            if (value is not null)
                SourceKind = value.Value;
        }
    }

    public Option<ulong>? SourceOption
    {
        get => SourceOptions.FirstOrDefault(option => option.Value == SourceId);
        set
        {
            if (value is not null)
                SourceId = value.Value;
        }
    }

    public Option<string>? EncoderOption
    {
        get => EncoderOptions.FirstOrDefault(option => option.Value == Encoder);
        set
        {
            if (value is not null)
                Encoder = value.Value;
        }
    }

    public Option<ulong>? AudioOption
    {
        get =>
            AudioMode == "Source"
                ? AudioOptions.FirstOrDefault(option => option.Value == SourceAudioValue)
                : AudioOptions.FirstOrDefault(option => option.Value == AudioUnitId);
        set
        {
            if (value is not null)
                if (value.Value == SourceAudioValue)
            {
                AudioMode = "Source";
                AudioUnitId = 0;
            }
            else
            {
                AudioMode = value.Value == 0 ? "Silent" : "Unit";
                AudioUnitId = value.Value;
            }
        }
    }

    public Option<int>? DriveOption
    {
        get => DriveOptions.FirstOrDefault(option => option.Value == DriveIndex);
        set
        {
            if (value is not null)
                DriveIndex = value.Value;
        }
    }

    public ulong SceneId => SourceKind == "Input" ? 0 : SourceId;

    public ulong InputId => SourceKind == "Input" ? SourceId : 0;

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Raises change notifications for the option lists the window replaced.</summary>
    public void NotifyOptions()
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(KindOptions)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(SourceOptions)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(EncoderOptions)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(AudioOptions)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(DriveOptions)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(KindOption)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(SourceOption)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(EncoderOption)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(AudioOption)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(DriveOption)));
    }

    private bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return false;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
        return true;
    }
}

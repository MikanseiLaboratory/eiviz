using System.Runtime.InteropServices;
using System.Windows;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Compact multi-angle view.
///
/// Every angle is its own scene, and each one shown on its own costs a present
/// surface. This composes them into a single multiview surface instead, so a
/// replay with many angles stays inside the flip budget and still shows all of
/// them. Labels come from the core's multiview styling, which is why the grid is
/// defined rather than drawn by the host.
/// </summary>
internal static class ReplayMultiview
{
    /// <summary>
    /// Id of the composed surface. It sits in the multiview band but above the ids
    /// a session uses, so it cannot collide with an operator's own multiview.
    /// </summary>
    public const ulong SceneId = 0x0002_7F00;

    /// <summary>Angles per row.</summary>
    public const int Columns = 3;

    /// <summary>Angles shown individually before the compact view takes over.</summary>
    public const int CompactThreshold = 4;

    /// <summary>
    /// Defines the composed surface for these angles. Returns false when there is
    /// nothing to show.
    /// </summary>
    public static bool Define(IReadOnlyList<ReplayAngleInfo> angles, uint width, uint height)
    {
        if (angles.Count == 0)
            return false;
        var rows = (angles.Count + Columns - 1) / Columns;
        var layers = new OverlayDesc[angles.Count];
        var labels = new List<GCHandle>();
        try
        {
            for (var index = 0; index < angles.Count; index++)
            {
                var column = index % Columns;
                var row = index / Columns;
                // The core draws the label, so the angle name travels with the
                // layer. The bytes are pinned until the define call returns.
                var bytes = System.Text.Encoding.UTF8.GetBytes(angles[index].Label + "\0");
                var pin = GCHandle.Alloc(bytes, GCHandleType.Pinned);
                labels.Add(pin);
                layers[index] = new OverlayDesc
                {
                    SourceId = angles[index].SceneGpuId,
                    Rect = new Interop.Rect
                    {
                        X = column / (float)Columns,
                        Y = row / (float)rows,
                        Width = 1f / Columns,
                        Height = 1f / rows
                    },
                    Opacity = 1.0f,
                    Z = index,
                    AudioFollow = 0,
                    Hidden = 0,
                    Label = pin.AddrOfPinnedObject(),
                    LayerId = (ulong)(index + 1)
                };
            }
            unsafe
            {
                fixed (OverlayDesc* pointer = layers)
                {
                    return MixerNative.DefineScene(
                        SceneId,
                        width,
                        height,
                        (uint)layers.Length,
                        pointer) == 0;
                }
            }
        }
        finally
        {
            foreach (var pin in labels)
            {
                if (pin.IsAllocated)
                    pin.Free();
            }
        }
    }

    /// <summary>
    /// Angle at a point on the composed surface, given as fractions of it. Returns
    /// null outside the grid.
    /// </summary>
    public static uint? SlotAt(double fractionX, double fractionY, int count)
    {
        if (count <= 0 || fractionX < 0.0 || fractionY < 0.0)
            return null;
        var rows = (count + Columns - 1) / Columns;
        var column = (int)(fractionX * Columns);
        var row = (int)(fractionY * rows);
        column = Math.Clamp(column, 0, Columns - 1);
        row = Math.Clamp(row, 0, rows - 1);
        var index = row * Columns + column;
        return index < count ? (uint)index : null;
    }
}


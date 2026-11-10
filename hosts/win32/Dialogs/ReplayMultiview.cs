using System;
using System.Windows;
using System.Collections.Generic;
using Eiviz.Host.Interop;
using Rect = Eiviz.Host.Interop.Rect;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Lays a set of sources out in a grid as one scene, so a deck can watch them through one surface.
/// </summary>
/// <remarks>
/// The angle tiles used to have a swapchain each. That is an expensive way to show a grid: every
/// surface costs a present every frame, and Direct3D runs out of them — attaching sixteen at once
/// lost the device on the machine this was measured on.
///
/// The mixer already composes a grid of layers into one texture; that is how a mixing unit's
/// multiview works. So the grid is built as a scene instead: one scene, one layer per angle, and one
/// monitor watching it. That is one swapchain however many angles there are, and the drawing, the
/// labels and the tallies come from the code that already does it.
///
/// The layout is a row-major grid. The column count is chosen to keep cells close to 16:9, because
/// the pictures are and square cells would letterbox every one of them.
/// </remarks>
internal static class ReplayMultiview
{
    /// <summary>
    /// Id of the scene the deck watches. It sits at the top of the scene band, clear of the ids a
    /// session's own scenes use.
    /// </summary>
    public const ulong SceneId = 0x0001_FFF0;

    private const uint SceneWidth = 1280;
    private const uint SceneHeight = 720;

    /// <summary>
    /// Geometry for a grid of <paramref name="count"/> tiles.
    /// </summary>
    public static (int Columns, int Rows) GridFor(int count)
    {
        if (count <= 0)
            return (1, 1);
        var columns = (int)Math.Ceiling(Math.Sqrt(count * 16.0 / 9.0));
        columns = Math.Clamp(columns, 1, count);
        return (columns, (int)Math.Ceiling(count / (double)columns));
    }

    /// <summary>
    /// Defines the grid scene for <paramref name="sources"/>, in the order they should appear.
    /// </summary>
    public static void Apply(IReadOnlyList<ulong> sources, IReadOnlyList<string> labels)
    {
        if (Application.Current is App { Backend.IsRemote: true })
            return;
        if (sources.Count == 0)
        {
            MixerApply.PushSceneLayers(SceneId, SceneWidth, SceneHeight, [], null);
            return;
        }
        var (columns, rows) = GridFor(sources.Count);
        // Layer rectangles are fractions of the scene, the same space `MultiviewPane` uses for the
        // built-in layouts. Pixels here would place every tile outside the surface.
        var cellWidth = 1f / columns;
        var cellHeight = 1f / rows;
        // A little air between tiles, so a grid of similar pictures does not read as one picture.
        const float Gap = 0.004f;
        var layers = new OverlayDesc[sources.Count];
        var texts = new string[sources.Count];
        for (var index = 0; index < sources.Count; index++)
        {
            var column = index % columns;
            var row = index / columns;
            layers[index] = new OverlayDesc
            {
                SourceId = sources[index],
                Rect = new Rect
                {
                    X = (column * cellWidth) + Gap,
                    Y = (row * cellHeight) + Gap,
                    Width = cellWidth - (Gap * 2),
                    Height = cellHeight - (Gap * 2),
                },
                Crop = new Rect { X = 0, Y = 0, Width = 1, Height = 1 },
                Opacity = 1.0f,
                Z = index,
                AudioFollow = 0,
                Hidden = 0,
                LayerId = (ulong)(index + 1),
            };
            texts[index] = labels.Count > index ? labels[index] : "";
        }
        MixerApply.PushSceneLayers(SceneId, SceneWidth, SceneHeight, layers, texts);
    }

    /// <summary>
    /// Removes the scene, so a closed deck leaves nothing behind.
    /// </summary>
    public static void Clear()
    {
        if (Application.Current is App { Backend.IsRemote: true })
            return;
        MixerApply.PushSceneLayers(SceneId, SceneWidth, SceneHeight, [], null);
    }
}

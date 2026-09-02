namespace Eiviz.Host.Interop;

internal enum TransitionGroup
{
    Basic,
    Wipe,
    Motion,
    Shader
}

internal readonly record struct TransitionInfo(
    uint Kind,
    string Label,
    TransitionGroup Group,
    bool HasDirection,
    bool HasDipColor,
    bool HasSoftness,
    bool HasParam,
    string SoftnessLabel,
    string ParamLabel);

internal static class TransitionCatalog
{
    internal static readonly TransitionInfo[] All =
    [
        new(MixerNative.TransitionCut, "Cut", TransitionGroup.Basic, false, false, false, false, "", ""),
        new(MixerNative.TransitionFade, "Fade", TransitionGroup.Basic, false, false, false, false, "", ""),
        new(MixerNative.TransitionDip, "Dip", TransitionGroup.Basic, false, true, false, false, "", ""),
        new(MixerNative.TransitionAdditive, "Additive", TransitionGroup.Basic, false, false, false, false, "", ""),
        new(MixerNative.TransitionCustom, "Custom WGSL", TransitionGroup.Basic, false, false, false, false, "", ""),
        new(MixerNative.TransitionWipe, "Wipe", TransitionGroup.Wipe, true, false, true, false, "Edge", ""),
        new(MixerNative.TransitionIris, "Iris", TransitionGroup.Wipe, false, false, true, false, "Edge", ""),
        new(MixerNative.TransitionBlinds, "Blinds", TransitionGroup.Wipe, true, false, true, true, "Edge", "Strips"),
        new(MixerNative.TransitionBarnDoor, "BarnDoor", TransitionGroup.Wipe, true, false, true, false, "Edge", ""),
        new(MixerNative.TransitionClock, "Clock", TransitionGroup.Wipe, true, false, true, false, "Edge", ""),
        new(MixerNative.TransitionHeart, "Heart", TransitionGroup.Wipe, false, false, true, false, "Edge", ""),
        new(MixerNative.TransitionDiamond, "Diamond", TransitionGroup.Wipe, false, false, true, false, "Edge", ""),
        new(MixerNative.TransitionStar, "Star", TransitionGroup.Wipe, false, false, true, false, "Edge", ""),
        new(MixerNative.TransitionRollerDoor, "RollerDoor", TransitionGroup.Wipe, true, false, true, false, "Edge", ""),
        new(MixerNative.TransitionSlide, "Slide", TransitionGroup.Motion, true, false, false, false, "", ""),
        new(MixerNative.TransitionPush, "Push", TransitionGroup.Motion, true, true, false, false, "", ""),
        new(MixerNative.TransitionZoom, "Zoom", TransitionGroup.Motion, false, false, false, false, "", ""),
        new(MixerNative.TransitionCrossZoom, "CrossZoom", TransitionGroup.Motion, false, false, false, false, "", ""),
        new(MixerNative.TransitionFlyRotate, "FlyRotate", TransitionGroup.Motion, true, false, false, true, "", "Spin"),
        new(MixerNative.TransitionFlip, "Flip", TransitionGroup.Motion, true, false, false, false, "", ""),
        new(MixerNative.TransitionCube, "Cube", TransitionGroup.Motion, true, false, false, false, "", ""),
        new(MixerNative.TransitionCubeZoom, "CubeZoom", TransitionGroup.Motion, true, false, false, false, "", ""),
        new(MixerNative.TransitionMultitask, "MultiTask", TransitionGroup.Motion, true, false, false, false, "", ""),
        new(MixerNative.TransitionLorez, "LoRez", TransitionGroup.Shader, false, false, true, false, "Pixel size", ""),
        new(MixerNative.TransitionMetamix, "MetaMix", TransitionGroup.Shader, false, false, false, true, "", "Copies"),
        new(MixerNative.TransitionTile, "Tile", TransitionGroup.Shader, true, false, false, true, "", "Tiles"),
        new(MixerNative.TransitionParts, "Parts", TransitionGroup.Shader, true, false, false, true, "", "Chunks"),
        new(MixerNative.TransitionStatic, "Static", TransitionGroup.Shader, false, false, true, true, "Edge", "Intensity"),
        new(MixerNative.TransitionShiftRgb, "Shift RGB", TransitionGroup.Shader, false, false, false, false, "", ""),
        new(MixerNative.TransitionDisplace, "Displace", TransitionGroup.Shader, false, false, false, true, "", "Intensity"),
        new(MixerNative.TransitionGlitch, "Glitch", TransitionGroup.Shader, false, true, true, true, "Edge", "Intensity"),
        new(MixerNative.TransitionSwirl, "Swirl", TransitionGroup.Shader, true, false, false, true, "", "Turns"),
        new(MixerNative.TransitionLumaMorph, "LumaMorph", TransitionGroup.Shader, false, false, true, false, "Edge", ""),
        new(MixerNative.TransitionRipple, "Ripple", TransitionGroup.Shader, false, false, false, true, "", "Intensity"),
        new(MixerNative.TransitionGridDissolve, "GridDissolve", TransitionGroup.Shader, false, false, true, true, "Edge", "Cells"),
        new(MixerNative.TransitionPolar, "Polar", TransitionGroup.Shader, true, false, false, false, "", ""),
        new(MixerNative.TransitionKaleidoscope, "Kaleidoscope", TransitionGroup.Shader, false, false, false, true, "", "Segments"),
        new(MixerNative.TransitionPageCurl, "PageCurl", TransitionGroup.Shader, true, false, false, false, "", ""),
        new(MixerNative.TransitionFilmBurn, "FilmBurn", TransitionGroup.Shader, false, true, true, true, "Edge", "Intensity"),
        new(MixerNative.TransitionZoomBlur, "ZoomBlur", TransitionGroup.Shader, false, false, false, true, "", "Intensity"),
        new(MixerNative.TransitionPixelSort, "PixelSort", TransitionGroup.Shader, true, false, true, true, "Threshold", "Span"),
        new(MixerNative.TransitionDatamosh, "Datamosh", TransitionGroup.Shader, true, false, false, true, "", "Intensity"),
        new(MixerNative.TransitionVisualDissolve, "VisualDissolve", TransitionGroup.Shader, false, false, true, true, "Edge", "Flow"),
        new(MixerNative.TransitionOpticalFlow, "OpticalFlow", TransitionGroup.Shader, false, false, false, true, "", "Amount"),
        new(MixerNative.TransitionBloom, "Bloom", TransitionGroup.Shader, false, false, true, true, "Threshold", "Intensity"),
    ];

    internal static TransitionInfo Info(uint kind) =>
        All.FirstOrDefault(item => item.Kind == kind, All[1]);

    internal static string Label(uint kind) => kind switch
    {
        MixerNative.TransitionStinger => "Stinger",
        _ => Info(kind).Label
    };

    internal static string GroupName(TransitionGroup group) => group switch
    {
        TransitionGroup.Wipe => "Wipe",
        TransitionGroup.Motion => "Motion",
        TransitionGroup.Shader => "Shader",
        _ => "Basic"
    };

    internal static float DefaultSoftness(uint kind) => kind switch
    {
        MixerNative.TransitionPixelSort => 0.4f,
        MixerNative.TransitionBloom => 0.45f,
        _ => 0.02f
    };

    internal static float DefaultParam(uint kind) => kind switch
    {
        MixerNative.TransitionPixelSort => 0.25f,
        MixerNative.TransitionDatamosh => 1f,
        MixerNative.TransitionMetamix => 8f,
        MixerNative.TransitionTile => 8f,
        MixerNative.TransitionParts => 6f,
        MixerNative.TransitionVisualDissolve => 0.42f,
        MixerNative.TransitionOpticalFlow => 1f,
        MixerNative.TransitionBloom => 0.85f,
        _ => 0f
    };

    internal static uint DefaultDurationValue(uint kind) => kind switch
    {
        MixerNative.TransitionPixelSort => 45,
        _ => 0
    };

    internal static uint? DefaultDirection(uint kind) => kind switch
    {
        MixerNative.TransitionPixelSort => 3u,
        _ => null
    };

    internal static bool ShowsSoftness(uint kind)
    {
        var label = Info(kind).SoftnessLabel;
        return !string.IsNullOrEmpty(label) && label != "Edge";
    }

    internal static void ApplyKindDefaults(TransitionPreset preset)
    {
        if (preset.Kind == MixerNative.TransitionPixelSort)
        {
            var oldPair = Math.Abs(preset.Softness - 0.3f) < 0.0005f && Math.Abs(preset.Param - 0.4f) < 0.0005f;
            if (oldPair)
            {
                preset.Softness = DefaultSoftness(preset.Kind);
                preset.Param = DefaultParam(preset.Kind);
                if (preset.DurationUnit == 0 && preset.DurationValue == 30)
                    preset.DurationValue = DefaultDurationValue(preset.Kind);
                if (preset.Direction == 0)
                    preset.Direction = DefaultDirection(preset.Kind) ?? 3u;
            }
            if (preset.Softness <= 0.021f)
                preset.Softness = DefaultSoftness(preset.Kind);
            if (preset.Param <= 0f)
                preset.Param = DefaultParam(preset.Kind);
        }
    }
}

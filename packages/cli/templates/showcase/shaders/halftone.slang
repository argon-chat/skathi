import sk;

// A print-style halftone of whatever is behind the element: an image, a video, the page. Every pixel
// of a cell reads the backdrop at the cell's centre, so a cell is one dot of one colour, sized by how
// bright that spot is.
[SkEffect("halftone")]
[SkDoc("re-screens the backdrop as rotated halftone dots")]
[SkBackdrop]
struct Halftone : IEffect
{
    [SkLength] [SkDefault("9")] [SkRange(3.0, 64.0)]
    float cell;

    // Screen angle in turns: 0.125 is the classic 45 degrees.
    [SkDefault("0.125")] [SkRange(-1.0, 1.0)]
    float angle;

    // How much colour the dots keep: 0 prints them in `paper`'s complement, 1 in the backdrop's own.
    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float colour;

    [SkColor] [SkOptional] [SkDefault("#0b0b12")]
    float4 paper;

    float4 shade(EffectInput input)
    {
        float turn = angle * 6.28318530718;
        float2 q = sk_rotate(input.px, -turn) / cell;
        float2 centre = floor(q) + 0.5;

        // Back to element pixels, kept inside the element so the edge cells read its own backdrop.
        float2 at = clamp(sk_rotate(centre * cell, turn), float2(0.0, 0.0), input.size);
        float2 uv = input.uv + (at - input.px) / sk_viewport_size;
        float4 seen = sk_backdrop_input.SampleLevel(sk_clamp_sampler, uv, 0.0);

        float3 rgb = seen.rgb / max(seen.a, 1e-4);
        float ink = sqrt(saturate(sk_luma(rgb) * 1.15)) * 0.5 * cell;
        float disc = input.aa(length(q - centre) * cell - ink);

        float3 tone = lerp(1.0 - paper.rgb, saturate(rgb * 1.2), colour);
        return float4(lerp(paper.rgb, tone, disc), 1.0);
    }
};

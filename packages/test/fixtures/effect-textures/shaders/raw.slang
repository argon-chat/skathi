import sk;

// Blue carries the image's width, so a test can read `size` off the screen.
[SkEffect("raw")]
struct Raw : IEffect
{
    [SkLinear]
    SkTexture image;

    float4 shade(EffectInput input)
    {
        float4 c = image.sample(input.local);
        return float4(c.r, c.g, image.size.x * 16.0 / 255.0, 1.0);
    }
};

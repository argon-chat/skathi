#Requires -Version 7.0

# Generates the imgperf test corpus into apps/imgperf/assets/: pwsh -File apps/imgperf/gen-assets.ps1
#
# Every image gets distinct bytes: gpui caches by path and its GPU atlas by decoded image id, so
# reusing one file would measure cache hits.
#
#   thumbs/t<NN>.png    50 x   64x64
#   photos/p<NN>.png    50 x  512x512
#   large/l<N>.png       4 x 2048x2048
#   large/l0.jpg         1 x 2048x2048  (JPEG decode path)
param([switch]$Force)

Add-Type -AssemblyName System.Drawing

$root = Join-Path $PSScriptRoot 'assets'
foreach ($d in 'thumbs', 'photos', 'large') {
    New-Item -ItemType Directory -Force (Join-Path $root $d) | Out-Null
}

function New-TestImage {
    param([int]$Size, [int]$Seed, [string]$Path, [string]$Format = 'Png')

    if ((Test-Path $Path) -and -not $Force) { return }

    $bmp = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias

    # Per-seed hue pair -> a diagonal gradient nobody else has.
    $h1 = ($Seed * 37) % 360
    $h2 = ($Seed * 91 + 140) % 360
    $c1 = [System.Drawing.Color]::FromArgb(255, (($h1 * 7) % 256), (($h1 * 13) % 256), (($h1 * 19) % 256))
    $c2 = [System.Drawing.Color]::FromArgb(255, (($h2 * 11) % 256), (($h2 * 5) % 256), (($h2 * 23) % 256))
    $rect = New-Object System.Drawing.Rectangle(0, 0, $Size, $Size)
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush($rect, $c1, $c2, 45.0)
    $g.FillRectangle($brush, $rect)

    # Rings: cheap high-frequency detail so the encoder cannot collapse the file
    # to a handful of bytes (a 2 KB "2048px image" would not measure a decode).
    $rings = 6 + ($Seed % 7)
    for ($i = 0; $i -lt $rings; $i++) {
        $r = [int]($Size * (0.08 + 0.42 * $i / $rings))
        $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(150, (($Seed * 29 + $i * 40) % 256), 255 - (($i * 30) % 256), (($Seed * 17 + $i * 60) % 256)), [Math]::Max(1, $Size / 64))
        $g.DrawEllipse($pen, ($Size / 2 - $r), ($Size / 2 - $r), (2 * $r), (2 * $r))
        $pen.Dispose()
    }
    # Pseudo-random speckle, seeded so runs are reproducible.
    $rand = New-Object System.Random($Seed)
    $dots = [int]($Size * 1.5)
    for ($i = 0; $i -lt $dots; $i++) {
        $x = $rand.Next(0, $Size); $y = $rand.Next(0, $Size)
        $sb = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb($rand.Next(60, 200), $rand.Next(256), $rand.Next(256), $rand.Next(256)))
        $g.FillRectangle($sb, $x, $y, [Math]::Max(1, $Size / 100), [Math]::Max(1, $Size / 100))
        $sb.Dispose()
    }

    $font = New-Object System.Drawing.Font('Segoe UI', [Math]::Max(8, $Size / 6), [System.Drawing.FontStyle]::Bold)
    $g.DrawString("$Seed", $font, [System.Drawing.Brushes]::White, 2, 2)
    $font.Dispose()

    $g.Dispose()
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::$Format)
    $bmp.Dispose()
    $brush.Dispose()
}

Write-Host 'thumbs 64px x50...'
for ($i = 0; $i -lt 50; $i++) {
    New-TestImage -Size 64 -Seed $i -Path (Join-Path $root ('thumbs\t{0:d2}.png' -f $i))
}
Write-Host 'photos 512px x50...'
for ($i = 0; $i -lt 50; $i++) {
    New-TestImage -Size 512 -Seed (100 + $i) -Path (Join-Path $root ('photos\p{0:d2}.png' -f $i))
}
Write-Host 'large 2048px x4 + 1 jpeg...'
for ($i = 0; $i -lt 4; $i++) {
    New-TestImage -Size 2048 -Seed (500 + $i) -Path (Join-Path $root ('large\l{0}.png' -f $i))
}
New-TestImage -Size 2048 -Seed 900 -Path (Join-Path $root 'large\l0.jpg') -Format 'Jpeg'

# One tiny SVG: gpui routes .svg through resvg, so it should just work.
$svg = @'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120" width="120" height="120">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#7fd4ff"/><stop offset="1" stop-color="#c47fff"/>
  </linearGradient></defs>
  <rect width="120" height="120" rx="18" fill="url(#g)"/>
  <circle cx="60" cy="52" r="26" fill="none" stroke="#fff" stroke-width="6"/>
  <path d="M22 104 L52 66 L74 92 L92 74 L104 104 Z" fill="#ffffffcc"/>
</svg>
'@
Set-Content -Path (Join-Path $root 'logo.svg') -Value $svg -Encoding UTF8

$files = Get-ChildItem -Recurse -File $root
$total = ($files | Measure-Object -Property Length -Sum).Sum
Write-Host ("done: {0} files, {1:n1} MB" -f $files.Count, ($total / 1MB))

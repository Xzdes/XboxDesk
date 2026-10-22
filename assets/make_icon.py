from PIL import Image, ImageDraw

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Темно-синий скругленный квадрат (фон как INK в приложении).
d.rounded_rectangle([32, 32, S - 32, S - 32], radius=224, fill=(24, 38, 61, 255))

# Зеленый круг Xbox со светлым кольцом.
cx = cy = S // 2
r = 352
d.ellipse(
    [cx - r, cy - r, cx + r, cy + r],
    fill=(16, 124, 16, 255),
    outline=(76, 175, 80, 255),
    width=20,
)

# Белая X с круглыми концами.
arm = 184
w = 132
d.line([cx - arm, cy - arm, cx + arm, cy + arm], fill=(255, 255, 255, 255), width=w)
d.line([cx - arm, cy + arm, cx + arm, cy - arm], fill=(255, 255, 255, 255), width=w)
cap = w // 2
for x, y in [(cx - arm, cy - arm), (cx + arm, cy + arm), (cx - arm, cy + arm), (cx + arm, cy - arm)]:
    d.ellipse([x - cap, y - cap, x + cap, y + cap], fill=(255, 255, 255, 255))

img = img.resize((256, 256), Image.LANCZOS)
img.save(
    r"S:\XboxDesk\assets\xboxdesk.ico",
    format="ICO",
    sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (96, 96), (128, 128), (256, 256)],
)
img.save(r"S:\XboxDesk\assets\icon_preview.png")
print("icon saved")

"""Génère assets/icon.png et assets/icon.ico : livre ouvert avec "A" et "文"."""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont, ImageFilter

out = Path(sys.argv[1])
out.mkdir(exist_ok=True)
S = 1024

# Fond : carré arrondi en dégradé indigo -> sarcelle
grad = Image.new("RGBA", (S, S))
top, bot = (67, 56, 202), (13, 148, 136)
px = grad.load()
for y in range(S):
    for x in range(S):
        t = (x * 0.35 + y * 0.65) / S
        px[x, y] = tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3)) + (255,)
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 40, S - 40), radius=210, fill=255)
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
img.paste(grad, (0, 0), mask)

# Ombre du livre
shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
sd = ImageDraw.Draw(shadow)
sd.polygon([(170, 340), (512, 395), (854, 340), (854, 800), (512, 865), (170, 800)], fill=(0, 0, 0, 90))
shadow = shadow.filter(ImageFilter.GaussianBlur(28))
img.alpha_composite(shadow, (0, 22))

d = ImageDraw.Draw(img)
# Pages : deux trapèzes légèrement courbés
left = [(160, 300), (300, 285), (430, 300), (512, 345), (512, 815), (430, 770), (300, 755), (160, 770)]
right = [(1024 - x, y) for x, y in left]
d.polygon(left, fill=(255, 255, 255, 255))
d.polygon(right, fill=(241, 245, 249, 255))
# Tranche centrale
d.line([(512, 345), (512, 815)], fill=(148, 163, 184, 255), width=10)
# Lignes de texte sur les pages
for i in range(4):
    y = 610 + i * 42
    d.line([(215, y - 8), (455, y + 10)], fill=(203, 213, 225, 255), width=14)
    d.line([(569, y + 10), (809, y - 8)], fill=(203, 213, 225, 255), width=14)

# Lettres : "A" à gauche, "文" à droite
fa = ImageFont.truetype(r"C:\Windows\Fonts\segoeuib.ttf", 250)
fz = ImageFont.truetype(r"C:\Windows\Fonts\msyhbd.ttc", 220)
d.text((335, 470), "A", font=fa, fill=(67, 56, 202, 255), anchor="mm")
d.text((690, 470), "文", font=fz, fill=(13, 148, 136, 255), anchor="mm")

# Flèche de traduction au-dessus du livre
d.line([(372, 190), (572, 190)], fill=(255, 255, 255, 255), width=34)
d.polygon([(562, 140), (652, 190), (562, 240)], fill=(255, 255, 255, 255))

img.resize((256, 256), Image.LANCZOS).save(out / "icon.png")
img.save(out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
img.resize((512, 512), Image.LANCZOS).save(out / "preview.png")
print("ok")


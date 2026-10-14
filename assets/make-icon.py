# Draws zeteme's icon, a "z" and a block cursor: assets/zeteme.png (1024 px)
# and assets/zeteme.ico. Needs Pillow; run from the repository root.
# start.html draws the same shapes as an SVG.
from PIL import Image, ImageDraw

SIZE = 1024
MINT = (80, 220, 170, 255)
GREY = (30, 30, 30, 255)
CURSOR = (230, 230, 230, 255)

im = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(im)
d.rounded_rectangle((32, 32, SIZE - 32, SIZE - 32), radius=200, fill=GREY)
left, right, top, bottom, stroke = 212, 662, 250, 770, 130
d.polygon([
    (left, top), (right, top), (right, top + stroke),
    (left + stroke * 1.25, bottom - stroke), (right, bottom - stroke),
    (right, bottom), (left, bottom), (left, bottom - stroke),
    (right - stroke * 1.25, top + stroke), (left, top + stroke),
], fill=MINT)
d.rectangle((right + 60, bottom - stroke, right + 170, bottom), fill=CURSOR)

im.save("assets/zeteme.png")
im.save("assets/zeteme.ico", sizes=[(s, s) for s in (256, 128, 64, 48, 32, 24, 16)])

"""Composed diagram → a Trellis image card. A worked example, and the method.

Run it (`python3 docs/diagram-recipe.py`) and it writes a PNG; the API calls to
turn that into a card are in API.md under **Diagrams**.

This produced the figure "The 4th dimension — a card with extent in time". It is
here because the *method* is reusable, not the drawing:

1. **Nothing is auto-laid-out.** Every box is placed by hand, so the composition
   is a decision. Graphviz and Mermaid answer "what connects to what"; they
   cannot answer "what does this mean", because the layout is whatever the
   algorithm picked. Use them when the content really is a graph.
2. **The layout is the argument.** Here the time axis *is* the point, and one bar
   spans four day-cells — so you see "this card has extent" before reading a
   word. Decide what the reader should understand, then let position carry it.
3. **A tight palette where every colour means something.** One background, one
   muted fill for context that is not the subject, three accents: the one card,
   its origin, what breaks. Nothing decorative.
4. **Four type sizes.** Title / label / body / caption, and captions sit beside
   what they annotate rather than in a legend.
5. **Ghost detail.** The faint strips inside each day carry no information but
   make a day read as a workspace with other things in it — which is the
   constraint being illustrated.
6. **State the failure.** The red line at the bottom says what is broken today.
   A diagram that only shows the happy path is a poster.

Keep the script beside the figure you post: an image card is pixels, and this is
the thing that can be corrected next month.
"""
from PIL import Image, ImageDraw, ImageFont

W, H = 1600, 900
BG = (15, 17, 21)
FG = (226, 232, 240)
DIM = (120, 132, 148)
ACCENT = (16, 185, 129)      # the task card
ORIGIN = (245, 158, 11)      # its origin day
GHOST = (30, 41, 59)
RULE = (51, 65, 85)

img = Image.new("RGB", (W, H), BG)
d = ImageDraw.Draw(img)


def font(sz, bold=False):
    p = ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf" if bold
         else "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    try:
        return ImageFont.truetype(p, sz)
    except OSError:
        return ImageFont.load_default()


F_T, F_H, F_B, F_S = font(38, True), font(23, True), font(19), font(16)

d.text((60, 40), "The 4th dimension: one card with extent in time", font=F_T, fill=FG)
d.text((60, 92), "A day is a 2-D space. Time is the axis it is missing. A card should occupy "
                 "a RANGE on that axis — not be copied onto it.", font=F_B, fill=DIM)

# ---- the day axis -----------------------------------------------------------
AX_Y = 300
d.line([(60, AX_Y), (W - 60, AX_Y)], fill=RULE, width=2)
d.text((W - 150, AX_Y - 34), "time →", font=F_S, fill=DIM)

days = ["8/11", "8/12", "8/13", "8/14", "8/15"]
x0, gap, bw, bh = 140, 290, 200, 260
boxes = []
for i, day in enumerate(days):
    x = x0 + i * gap
    y = AX_Y + 40
    boxes.append((x, y))
    d.rounded_rectangle([x, y, x + bw, y + bh], radius=10, outline=RULE, width=2, fill=GHOST)
    d.text((x + 12, y + 10), day, font=F_H, fill=FG)
    d.line([(x + 12, y + 40), (x + bw - 12, y + 40)], fill=RULE, width=1)
    d.ellipse([(x + (bw / 2) - 5, AX_Y - 5), (x + (bw / 2) + 5, AX_Y + 5)], fill=RULE)
    # each day's own cards — the context that must not be lost
    for r in range(2):
        d.rounded_rectangle([x + 14, y + 56 + r * 26, x + bw - 14, y + 74 + r * 26],
                            radius=4, fill=(44, 52, 66))

# ---- the task card, spanning days -------------------------------------------
sx, sy = boxes[0]
ex, _ = boxes[3]
bar_y = sy + 150
d.rounded_rectangle([sx + 14, bar_y, ex + bw - 14, bar_y + 54], radius=10,
                    fill=(6, 78, 59), outline=ACCENT, width=3)
d.text((sx + 30, bar_y + 15), "▣  one card — same id, same position, one truth",
       font=F_B, fill=(167, 243, 208))

# origin marker
d.rounded_rectangle([sx + 14, bar_y - 4, sx + bw - 14, bar_y + 58], radius=10,
                    outline=ORIGIN, width=3)
d.text((sx + 14, sy + bh + 20), "origin day — kept forever;", font=F_S, fill=ORIGIN)
d.text((sx + 14, sy + bh + 42), "step back and that whole day", font=F_S, fill=ORIGIN)
d.text((sx + 14, sy + bh + 64), "is still around it", font=F_S, fill=ORIGIN)

d.text((ex + bw - 190, sy + bh + 20), "due — span ends", font=F_S, fill=ACCENT)

# ---- the rule ---------------------------------------------------------------
d.text((60, 730), "start:: 8/11    due:: 8/14", font=F_H, fill=FG)
d.text((60, 768), "Stand in any day inside the span and the card is THERE, on the canvas, in its own "
                  "place — not a list row, not a copy.", font=F_B, fill=DIM)
d.text((60, 800), "Editing it in 8/13 edits the card in 8/11, because there is only one. "
                  "Leave the span and the day is unchanged.", font=F_B, fill=DIM)
d.text((60, 840), "What breaks today: a card lives in exactly ONE basket, so moving it to Open Items "
                  "cut it off from its day —", font=F_B, fill=(248, 113, 113))
d.text((60, 868), "and copying it forward made a second task. The span is what replaces both.",
       font=F_B, fill=(248, 113, 113))

img.save("/tmp/fig.png")
print("written")

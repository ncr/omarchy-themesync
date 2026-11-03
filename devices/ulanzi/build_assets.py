"""Compile deliberate native pixel templates, with no resizing or dithering.

The full scene is 52x16. Three cached 16x16 landscape tiles cover x=0..47;
the Berry script draws the 52-pixel ground and five 8x8 beds through x=50.
ImageGen sheets in art/ are preserved design references, not downsampled inputs.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent
TREE = [
    "....AA.....",
    "...AAAB....",
    "..AABBBB...",
    ".AABBBBBA..",
    "AABBBBBCBA.",
    "ABBBBCCCBA.",
    ".ABBCCCBA..",
    "..ABBBBA...",
    "....TT.....",
    "...TTT.....",
    "....TT.....",
    "....TT.....",
    "....TT.....",
    "...TTTT....",
]
WINTER = [
    "..S....S...",
    "...T..T....",
    "...T.T.....",
    "....TT..S..",
    ".S..TT.T...",
    "..T.TTT....",
    "...TTT.....",
    "....TT.....",
    "....TT.....",
    "...TTT.....",
    "....TT.....",
    "....TT.....",
    "....TT.....",
    "...TTTT....",
]
PALETTES = [
    {"A": (80,255,0), "B": (0,170,0), "C": (0,80,0), "T": (200,70,0), "P": (255,0,120)},
    {"A": (0,240,0), "B": (0,150,0), "C": (0,65,0), "T": (200,70,0), "P": (255,0,0)},
    {"A": (255,160,0), "B": (255,60,0), "C": (180,0,0), "T": (180,60,0)},
    {"S": (0,210,255), "T": (170,70,0)},
]
PLANTS = [
    ["..YYY...", ".YYYYY..", ".YYOYY..", "..YYY...", "...g....", ".LLgLL..", "...g....", "..ggg..."],
    [".R.R.R..", ".RRRRR..", "..RRR...", "...R....", "...g.L..", "...gLL..", "...g....", "..ggg..."],
    ["...V....", "..VVV...", ".VVPVV..", ".VV.VV..", "...g....", ".LLg....", "...gL...", "..ggg..."],
    ["........", "........", "........", "........", "..L.L...", ".LLgLL..", "...g....", "..ggg..."],
    ["........", "..C.C...", ".CC.CC..", "..CVC...", "..V.V...", "........", "........", "........"],
]
PLANT_COLOURS = {"Y":(255,220,0), "O":(255,80,0), "R":(255,0,0),
                 "V":(140,0,255), "P":(255,0,160), "L":(80,255,0),
                 "g":(0,160,0), "C":(0,220,255)}


def pixels(pattern, palette, size):
    assert len(pattern) <= size[1]
    assert all(len(row) == len(pattern[0]) for row in pattern)
    image = Image.new("RGB", size)
    for y, row in enumerate(pattern):
        for x, key in enumerate(row):
            if key != ".":
                image.putpixel((x,y), palette[key])
    return image


frames = []
for season, palette in enumerate(PALETTES):
    pattern = list(WINTER if season == 3 else TREE)
    if season in (0,1):
        for x,y in [(3,4),(7,6)]:
            pattern[y] = pattern[y][:x] + "P" + pattern[y][x+1:]
    for dusk in (False,True):
        colours = {key: tuple(int(channel * (0.65 if dusk else 1)) for channel in colour)
                   for key,colour in palette.items()}
        # In dusk, remove the darkest foliage cluster instead of adding a grey sky.
        if dusk and "C" in colours:
            colours["C"] = (0,0,0)
        frames.append(pixels(pattern,colours,(48,16)))
(root/"assets/garden.rgb").write_bytes(b"".join(frame.tobytes() for frame in frames))
for index, frame in enumerate(frames):
    frame.save(root/f"assets/garden-{index}.png")
sprites = [pixels(pattern,PLANT_COLOURS,(8,8)) for pattern in PLANTS]
(root/"assets/plants.rgb").write_bytes(b"".join(sprite.tobytes() for sprite in sprites))
for index,sprite in enumerate(sprites):
    sprite.save(root/f"assets/plant-{index}.png")

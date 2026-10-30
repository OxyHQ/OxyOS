"""Frames: QEMU's PPM screendumps, the theme's PNGs, and what a frame shows. Standard library only."""

import struct
import zlib


class Frame:
    """An RGB picture, 8 bits a channel, rows top to bottom."""

    def __init__(self, width, height, rgb):
        self.width, self.height, self.rgb = width, height, bytes(rgb)
        if len(self.rgb) != width * height * 3:
            raise ValueError(f"{width}x{height} frame with {len(self.rgb)} bytes")

    @property
    def size(self):
        return f"{self.width}x{self.height}"

    def is_black(self, threshold=0):
        """Every channel of every pixel at most `threshold`: nothing drawn, not text, not a cursor."""
        return max(self.rgb) <= threshold if self.rgb else True

    def lit(self, threshold=0):
        """The bounding box (x0, y0, x1, y1) of pixels brighter than `threshold`, or None."""
        row_bytes = self.width * 3
        rows = [y for y in range(self.height) if max(self.rgb[y * row_bytes:(y + 1) * row_bytes]) > threshold]
        if not rows:
            return None
        columns = []
        for y in rows:
            row = self.rgb[y * row_bytes:(y + 1) * row_bytes]
            columns.extend(i // 3 for i, value in enumerate(row) if value > threshold)
        return (min(columns), rows[0], max(columns) + 1, rows[-1] + 1)

    def difference(self, other):
        """(pixels that differ, largest channel difference) between two frames of one size."""
        if (self.width, self.height) != (other.width, other.height):
            return (self.width * self.height, 255)
        if self.rgb == other.rgb:
            return (0, 0)
        pixels, largest = 0, 0
        a, b = self.rgb, other.rgb
        for i in range(0, len(a), 3):
            if a[i:i + 3] != b[i:i + 3]:
                pixels += 1
                largest = max(largest, abs(a[i] - b[i]), abs(a[i + 1] - b[i + 1]), abs(a[i + 2] - b[i + 2]))
        return (pixels, largest)

    def png(self):
        """This frame as a PNG (filter 0 rows, deflated): small for a mostly black screen."""
        raw = bytearray()
        row_bytes = self.width * 3
        for y in range(self.height):
            raw.append(0)
            raw += self.rgb[y * row_bytes:(y + 1) * row_bytes]

        def chunk(kind, data):
            body = kind + data
            return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

        header = struct.pack(">IIBBBBB", self.width, self.height, 8, 2, 0, 0, 0)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
                + chunk(b"IEND", b""))


def read_ppm(path):
    """A binary PPM (P6, maxval 255), as QEMU's screendump writes it."""
    data = path.read_bytes()
    fields, position = [], 0
    while len(fields) < 4:
        while data[position:position + 1].isspace():
            position += 1
        if data[position:position + 1] == b"#":
            position = data.index(b"\n", position) + 1
            continue
        end = position
        while not data[end:end + 1].isspace():
            end += 1
        fields.append(data[position:end])
        position = end
    magic, width, height, maxval = fields[0], int(fields[1]), int(fields[2]), int(fields[3])
    if magic != b"P6" or maxval != 255:
        raise ValueError(f"{path}: not an 8-bit P6 PPM")
    position += 1
    return Frame(width, height, data[position:position + width * height * 3])


def read_png(path):
    """An 8-bit, non-interlaced PNG (grey, RGB, grey+alpha or RGBA) composited over black, as
    Plymouth draws the theme's images on its black background."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")
    position, idat, header = 8, bytearray(), None
    while position < len(data):
        length, kind = struct.unpack(">I4s", data[position:position + 8])
        body = data[position + 8:position + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        position += 12 + length
    width, height, depth, colour, _, _, interlace = header
    channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(colour)
    if depth != 8 or channels is None or interlace:
        raise ValueError(f"{path}: unsupported PNG (depth {depth}, colour type {colour}, interlace {interlace})")
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    previous = bytearray(stride)
    rgb = bytearray()
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            corner = previous[i - channels] if i >= channels else 0
            if kind == 1:
                line[i] = (line[i] + left) & 0xFF
            elif kind == 2:
                line[i] = (line[i] + up) & 0xFF
            elif kind == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xFF
            elif kind == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                predictor = left if pa <= pb and pa <= pc else up if pb <= pc else corner
                line[i] = (line[i] + predictor) & 0xFF
        previous = line
        for x in range(width):
            pixel = line[x * channels:(x + 1) * channels]
            if channels == 1:
                rgb += bytes((pixel[0],)) * 3
            elif channels == 2:
                rgb += bytes(((pixel[0] * pixel[1] + 127) // 255,)) * 3
            elif channels == 3:
                rgb += pixel
            else:
                alpha = pixel[3]
                rgb += bytes((c * alpha + 127) // 255 for c in pixel[:3])
    return Frame(width, height, rgb)


def centred_on_black(image, width, height):
    """`image` where the oxyos theme puts it on a display of width x height: centred, its
    margins whole (Math.Int((W - w) / 2)), the rest black."""
    left, top = (width - image.width) // 2, (height - image.height) // 2
    rgb = bytearray(width * height * 3)
    row = image.width * 3
    for y in range(image.height):
        start = ((top + y) * width + left) * 3
        rgb[start:start + row] = image.rgb[y * row:(y + 1) * row]
    return Frame(width, height, rgb)

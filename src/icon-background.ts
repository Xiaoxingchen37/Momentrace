const WHITE = "#ffffff";
const CACHE_LIMIT = 256;
/** Colours are grouped into 16 levels per channel so anti-aliased edges of the
 * same flat background collapse into one bucket. */
const QUANTIZE_SHIFT = 4;
/** Rounded tiles cut their corners away, so a border that is roughly two thirds
 * solid still counts as a filled tile; free-form glyphs fall far below this. */
const MIN_OPAQUE_SHARE = 0.5;
/** Share of the solid border that must agree before the colour is trusted. */
const DOMINANT_SHARE = 0.85;
const OPAQUE_ALPHA = 224;
const NEAR_WHITE = 246;

const cache = new Map<string, Promise<string>>();

const remember = (source: string, value: Promise<string>) => {
  if (cache.size >= CACHE_LIMIT) {
    const oldest = cache.keys().next();
    if (!oldest.done) cache.delete(oldest.value);
  }
  cache.set(source, value);
};

const toHex = (value: number) =>
  Math.round(value).toString(16).padStart(2, "0");

type Pixels = { data: Uint8ClampedArray; width: number; height: number };

const readPixels = (image: HTMLImageElement): Pixels | undefined => {
  const canvas = document.createElement("canvas");
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  if (!canvas.width || !canvas.height) return undefined;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return undefined;
  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
  return { data, width: canvas.width, height: canvas.height };
};

const alphaAt = ({ data, width }: Pixels, x: number, y: number) =>
  data[(y * width + x) * 4 + 3];

/** Trims fully transparent padding so the border ring follows the artwork
 * rather than the canvas, which is what makes flat-square icons detectable. */
const opaqueBounds = (pixels: Pixels) => {
  let left = pixels.width;
  let right = -1;
  let top = pixels.height;
  let bottom = -1;
  for (let y = 0; y < pixels.height; y += 1) {
    for (let x = 0; x < pixels.width; x += 1) {
      if (alphaAt(pixels, x, y) < OPAQUE_ALPHA) continue;
      if (x < left) left = x;
      if (x > right) right = x;
      if (y < top) top = y;
      if (y > bottom) bottom = y;
    }
  }
  return right < left || bottom < top ? undefined : { left, right, top, bottom };
};

type Bucket = { count: number; total: [number, number, number] };

const dominantBorderColor = (pixels: Pixels) => {
  const bounds = opaqueBounds(pixels);
  if (!bounds) return WHITE;

  const { left, right, top, bottom } = bounds;
  const width = right - left + 1;
  const height = bottom - top + 1;
  if (width < 8 || height < 8) return WHITE;

  const buckets = new Map<number, Bucket>();
  let sampled = 0;
  let transparent = 0;

  const sample = (x: number, y: number) => {
    const offset = (y * pixels.width + x) * 4;
    sampled += 1;
    if (pixels.data[offset + 3] < OPAQUE_ALPHA) {
      transparent += 1;
      return;
    }
    const red = pixels.data[offset];
    const green = pixels.data[offset + 1];
    const blue = pixels.data[offset + 2];
    const key =
      ((red >> QUANTIZE_SHIFT) << 8) |
      ((green >> QUANTIZE_SHIFT) << 4) |
      (blue >> QUANTIZE_SHIFT);
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.count += 1;
      bucket.total[0] += red;
      bucket.total[1] += green;
      bucket.total[2] += blue;
    } else {
      buckets.set(key, { count: 1, total: [red, green, blue] });
    }
  };

  for (let x = left; x <= right; x += 1) {
    sample(x, top);
    sample(x, bottom);
  }
  for (let y = top + 1; y < bottom; y += 1) {
    sample(left, y);
    sample(right, y);
  }

  // A border that is mostly cut out is a free-form glyph, not a filled tile.
  const opaque = sampled - transparent;
  if (opaque < sampled * MIN_OPAQUE_SHARE) return WHITE;

  let dominant: Bucket | undefined;
  for (const bucket of buckets.values()) {
    if (!dominant || bucket.count > dominant.count) dominant = bucket;
  }
  // Measured against the solid part of the border so that rounded corners,
  // which are transparent by design, do not veto an otherwise flat tile.
  if (!dominant || dominant.count < opaque * DOMINANT_SHARE) return WHITE;

  const channels = dominant.total.map((value) => value / dominant.count);
  if (channels.every((value) => value >= NEAR_WHITE)) return WHITE;
  return `#${channels.map(toHex).join("")}`;
};

export const getIconBackground = (source: string) => {
  const cached = cache.get(source);
  if (cached) return cached;

  const result = new Promise<string>((resolve) => {
    const image = new Image();
    image.onload = () => {
      try {
        const pixels = readPixels(image);
        resolve(pixels ? dominantBorderColor(pixels) : WHITE);
      } catch {
        resolve(WHITE);
      }
    };
    image.onerror = () => resolve(WHITE);
    image.src = source;
  });
  remember(source, result);
  return result;
};

const WHITE = "#ffffff";
const backgroundCache = new Map<string, Promise<string>>();

const toHex = (value: number) =>
  Math.round(value).toString(16).padStart(2, "0");

const analyzeEdge = (image: HTMLImageElement) => {
  const canvas = document.createElement("canvas");
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context || !canvas.width || !canvas.height) return WHITE;

  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
  const minimum = [255, 255, 255];
  const maximum = [0, 0, 0];
  const total = [0, 0, 0];
  let count = 0;

  const addPixel = (x: number, y: number) => {
    const offset = (y * canvas.width + x) * 4;
    if (data[offset + 3] < 200) return false;
    for (let channel = 0; channel < 3; channel += 1) {
      const value = data[offset + channel];
      minimum[channel] = Math.min(minimum[channel], value);
      maximum[channel] = Math.max(maximum[channel], value);
      total[channel] += value;
    }
    count += 1;
    return true;
  };

  for (let y = 0; y < canvas.height; y += 1) {
    for (let x = 0; x < canvas.width; x += 1) {
      if (addPixel(x, y)) break;
    }
    for (let x = canvas.width - 1; x >= 0; x -= 1) {
      if (addPixel(x, y)) break;
    }
  }
  for (let x = 0; x < canvas.width; x += 1) {
    for (let y = 0; y < canvas.height; y += 1) {
      if (addPixel(x, y)) break;
    }
    for (let y = canvas.height - 1; y >= 0; y -= 1) {
      if (addPixel(x, y)) break;
    }
  }

  if (!count || maximum.some((value, channel) => value - minimum[channel] > 20)) {
    return WHITE;
  }
  return `#${total.map((value) => toHex(value / count)).join("")}`;
};

export const getIconBackground = (source: string) => {
  const cached = backgroundCache.get(source);
  if (cached) return cached;

  const result = new Promise<string>((resolve) => {
    const image = new Image();
    image.onload = () => {
      try {
        resolve(analyzeEdge(image));
      } catch {
        resolve(WHITE);
      }
    };
    image.onerror = () => resolve(WHITE);
    image.src = source;
  });
  backgroundCache.set(source, result);
  return result;
};

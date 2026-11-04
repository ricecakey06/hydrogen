export function skinLayout(width: number, height: number) {
  if (!Number.isInteger(width) || width < 64 || width > 1024 || width % 64 !== 0 || (height !== width && height !== width / 2)) {
    throw new Error('无效的 Minecraft 皮肤尺寸');
  }
  return { scale: width / 64, legacy: height === width / 2 };
}

export function legacyHatIsVisible(pixels: Uint8ClampedArray) {
  // Old RGB skins fill unused hat pixels with an opaque color. Minecraft ignores that layer.
  for (let i = 3; i < pixels.length; i += 4) if (pixels[i] < 128) return true;
  return false;
}

export function skinHead(image: HTMLImageElement): string {
  const { scale, legacy } = skinLayout(image.naturalWidth, image.naturalHeight);
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 128;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('无法绘制玩家头像');
  context.imageSmoothingEnabled = false;
  context.drawImage(image, 8 * scale, 8 * scale, 8 * scale, 8 * scale, 0, 0, 128, 128);
  let showHat = true;
  if (legacy) {
    const overlay = document.createElement('canvas');
    overlay.width = overlay.height = 32 * scale;
    const overlayContext = overlay.getContext('2d');
    if (!overlayContext) throw new Error('无法读取皮肤图层');
    overlayContext.drawImage(image, 32 * scale, 0, 32 * scale, 32 * scale, 0, 0, overlay.width, overlay.height);
    showHat = legacyHatIsVisible(overlayContext.getImageData(0, 0, overlay.width, overlay.height).data);
  }
  if (showHat) context.drawImage(image, 40 * scale, 8 * scale, 8 * scale, 8 * scale, 0, 0, 128, 128);
  return canvas.toDataURL('image/png');
}

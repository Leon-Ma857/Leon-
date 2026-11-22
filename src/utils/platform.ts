const platformRaw =
  typeof navigator !== "undefined"
    ? `${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`
    : "";

export const IS_MAC = /mac|darwin/i.test(platformRaw);
export const MOD_LABEL = IS_MAC ? "⌘" : "Ctrl";
export const ALT_LABEL = IS_MAC ? "Option" : "Alt";
export const OS_LABEL = IS_MAC ? "macOS" : "Windows";
export const NATIVE_OCR_NAME = IS_MAC ? "Apple Vision OCR" : "Windows OCR";
export const NATIVE_OCR_FULL_NAME = IS_MAC ? "Apple Vision 系统 OCR" : "Windows 原生 OCR";

export function formatShortcutLabel(shortcut: string): string {
  if (!IS_MAC) return shortcut;
  return shortcut
    .replace(/\bAlt\b/gi, "Option")
    .replace(/\bSuper\b/gi, "⌘")
    .replace(/\bCommandOrControl\b/gi, "⌘");
}

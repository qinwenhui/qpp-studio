/** 主题应用。glass 主题的系统材质(Mica/Acrylic)由 Rust 侧在 settings_set 时设置。 */

export const THEMES = [
  { id: 'dark-tech', label: '深色科技', swatch: 'linear-gradient(135deg,#0e1116,#1a212c 60%,#2dd4a7)' },
  { id: 'light-refined', label: '浅色精致', swatch: 'linear-gradient(135deg,#f5f3ee,#fdfcfa 60%,#0e9270)' },
  { id: 'macos-glass', label: '通透玻璃', swatch: 'linear-gradient(135deg,#4ea0ff88,#2a2e38 60%,#74b4ff)' },
  { id: 'cute', label: '可爱粉彩', swatch: 'linear-gradient(135deg,#fdf2f6,#fffafc 60%,#f06292)' },
  { id: 'classical', label: '古典纸墨', swatch: 'linear-gradient(135deg,#f2ecdd,#f9f5ea 60%,#8a5a2b)' },
] as const;

export function applyTheme(theme: string) {
  document.documentElement.setAttribute('data-theme', theme);
}

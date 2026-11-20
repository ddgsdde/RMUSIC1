import type { ThemeManifest } from "@r-music/sdk";
export const androidThemes: ThemeManifest[] = [
  {
    schema_version: 1,
    id: "graphite",
    name: "黑白",
    dark: {
      bg: "#000000",
      surface: "#000000",
      raised: "#141414",
      text: "#f4f4f5",
      muted: "#a1a1aa",
      border: "#27272a",
      accent: "#cee568",
      accentText: "#1b210c",
    },
    light: {
      bg: "#ffffff",
      surface: "#ffffff",
      raised: "#f4f4f5",
      text: "#202024",
      muted: "#66666f",
      border: "#e4e4e7",
      accent: "#5a7116",
      accentText: "#ffffff",
    },
  },
  {
    schema_version: 1,
    id: "paper",
    name: "纸感",
    light: {
      bg: "#fbfaf7",
      surface: "#ffffff",
      raised: "#efeee9",
      text: "#262724",
      muted: "#777970",
      border: "#e2e2db",
      accent: "#496f57",
      accentText: "#ffffff",
    },
    dark: {
      bg: "#20211e",
      surface: "#292a26",
      raised: "#343630",
      text: "#f0f0e8",
      muted: "#afb2a3",
      border: "#41433b",
      accent: "#c5d4ae",
      accentText: "#24321b",
    },
  },
  {
    schema_version: 1,
    id: "ocean",
    name: "海蓝",
    dark: {
      bg: "#101921",
      surface: "#182633",
      raised: "#223646",
      text: "#eaf2f7",
      muted: "#99afc0",
      border: "#2b3d4b",
      accent: "#9acded",
      accentText: "#102a3c",
    },
    light: {
      bg: "#f3f8fc",
      surface: "#ffffff",
      raised: "#e6f0f6",
      text: "#183249",
      muted: "#627e93",
      border: "#d6e5ef",
      accent: "#30769e",
      accentText: "#ffffff",
    },
  },
  {
    schema_version: 1,
    id: "sand",
    name: "暖砂",
    dark: {
      bg: "#211b17",
      surface: "#2a231e",
      raised: "#382f27",
      text: "#f7eee5",
      muted: "#baa99a",
      border: "#44382e",
      accent: "#e7bc8d",
      accentText: "#3c2817",
    },
    light: {
      bg: "#faf5ed",
      surface: "#fffcf7",
      raised: "#eee3d5",
      text: "#382c21",
      muted: "#8a7765",
      border: "#e3d5c4",
      accent: "#967044",
      accentText: "#ffffff",
    },
  },


  {schema_version:1,id:"liser-violet",name:"暮光紫",
   dark:{bg:"#171321",surface:"#211b2c",raised:"#30273e",text:"#f5efff",muted:"#b6a9c7",border:"#41354f",accent:"#cdb4ff",accentText:"#302044"},
   light:{bg:"#f8f4fc",surface:"#fffbff",raised:"#eee5f7",text:"#2b2135",muted:"#756580",border:"#e2d7eb",accent:"#7244b3",accentText:"#ffffff"}},
  {schema_version:1,id:"liser-blue",name:"夜航蓝",
   dark:{bg:"#111923",surface:"#1a2635",raised:"#26364b",text:"#eef4ff",muted:"#a2b4ce",border:"#36465c",accent:"#abcaff",accentText:"#172d4e"},
   light:{bg:"#f3f7fe",surface:"#fcfdff",raised:"#e2ebfa",text:"#20324c",muted:"#61738d",border:"#d5e0ef",accent:"#365f9c",accentText:"#ffffff"}},
  {schema_version:1,id:"liser-emerald",name:"雾林绿",
   dark:{bg:"#111d1a",surface:"#1a2a25",raised:"#263c33",text:"#edf8f0",muted:"#a2bcb0",border:"#354e42",accent:"#a3dfbf",accentText:"#173d2a"},
   light:{bg:"#f3f9f5",surface:"#fcfffd",raised:"#e1eee5",text:"#203c2d",muted:"#637f6e",border:"#d4e4da",accent:"#356c4e",accentText:"#ffffff"}},
];
const colorKeys = [
  "bg",
  "surface",
  "raised",
  "text",
  "muted",
  "border",
  "accent",
  "accentText",
];
export function validateTheme(value: unknown): ThemeManifest {
  if (!value || typeof value !== "object")
    throw new Error("主题必须为 JSON 对象");
  const item = value as ThemeManifest;
  if (
    item.schema_version !== 1 ||
    !/^[-a-z0-9]{1,48}$/.test(item.id) ||
    typeof item.name !== "string" ||
    item.name.length > 40
  )
    throw new Error("主题版本、名称或 ID 无效");
  for (const mode of ["light", "dark"] as const)
    for (const key of colorKeys)
      if (!/^#[0-9a-f]{6}$/i.test(item[mode]?.[key] ?? ""))
        throw new Error(`主题 ${mode}.${key} 必须为六位十六进制颜色`);
  return {
    schema_version: 1,
    id: item.id,
    name: item.name,
    light: Object.fromEntries(colorKeys.map((key) => [key, item.light[key]])),
    dark: Object.fromEntries(colorKeys.map((key) => [key, item.dark[key]])),
  };
}
export function applyTheme(theme: ThemeManifest, mode: "light" | "dark") {
  document.documentElement.dataset.theme = theme.id;
  for (const [key, value] of Object.entries(theme[mode]))
    document.documentElement.style.setProperty(`--${key}`, value);
  document.documentElement.dataset.mode = mode;
  document.documentElement.style.colorScheme = mode;
}

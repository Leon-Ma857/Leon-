export type TranslateEngine = "ai" | "google" | "deepl";
export type ResultMode = "translate" | "ocr" | "ask" | "pin";
export type StitchDirection = "vertical" | "horizontal" | "grid";
export type ThemeMode = "system" | "light" | "dark";
export type LocalOcrEngineMode = "auto" | "rapidocr" | "windows";
export type LocalOcrLang = "ch" | "cht" | "en" | "jp" | "kr" | "ru";

export interface SystemProxyInfo {
  enabled: boolean;
  proxyUrl: string | null;
  source: "env" | "windows" | "macos" | "none";
}

export interface AskQuickAction {
  id: string;
  label: string;
  prompt: string;
}

export const DEFAULT_ASK_QUICK_ACTIONS: AskQuickAction[] = [
  {
    id: "md-table",
    label: "转表格",
    prompt: "请将内容中的表格或结构化数据精准提取并转换为规范的 Markdown 表格，保留完整表头、数字与对齐关系。",
  },
  {
    id: "latex-formula",
    label: "提取公式",
    prompt: "请精准识别并提取内容中的数学公式，输出可直接复制使用的 LaTeX 代码（行内用 $...$，独立公式用 $$...$$），并简要核对符号。",
  },
  {
    id: "explain-error",
    label: "解释报错",
    prompt: "请分析这段报错信息或代码异常：说明根本原因、关键上下文线索，并给出具体可执行的排查与修复步骤。",
  },
  {
    id: "summarize-key",
    label: "总结要点",
    prompt: "请提炼核心结论与关键要点，按条理清晰的要点列表输出，突出重要数据、动作项或结论。",
  },
  {
    id: "explain-code",
    label: "解释代码",
    prompt: "请清晰解释这段代码的核心逻辑、输入输出与关键步骤，如有明显潜在 Bug 或边界问题一并指出。",
  },
  {
    id: "polish-rewrite",
    label: "润色改写",
    prompt: "请在不改变原意的前提下，对这段文字进行专业、自然、简洁的润色改写，并保留关键术语。",
  },
];

export interface LocalOcrStatus {
  installed: boolean;
  version: string;
  installDir: string;
  diskBytes: number;
  availableLangs: string[];
  activeLang: string;
  busy: boolean;
  lastError?: string | null;
}

export interface LocalOcrProgress {
  stage: "downloading" | "verifying" | "extracting" | "ready" | "error" | "cancelled";
  downloaded: number;
  total?: number | null;
  percent?: number | null;
  message?: string | null;
}

export interface AppConfig {
  shortcut: string;
  clipboard_shortcut: string;
  selection_shortcut: string;
  target_lang: string;
  translate_engine: TranslateEngine;
  google_api_key: string;
  deepl_api_key: string;
  api_base_url: string;
  api_key: string;
  model: string;
  custom_prompt: string;
  glossary: string;
  auto_translate_on_release: boolean;
  start_minimized: boolean;
  launch_at_startup: boolean;
  max_stack_size: number;
  clear_stack_after_output: boolean;
  theme: ThemeMode;
  custom_prompt_presets?: string[];
  ask_quick_actions: AskQuickAction[];
  ocr_engine: LocalOcrEngineMode;
  ocr_lang: LocalOcrLang;
  ocr_download_url: string;
  proxy_url: string;
  update_feed_url: string;
}

export interface OverlayPayload {
  captureId: number;
  monitorWidth: number;
  monitorHeight: number;
  scaleFactor: number;
  autoTranslateOnRelease: boolean;
  translateEngine: TranslateEngine;
  targetLang: string;
}

export interface AiChatMessage {
  role: "user" | "assistant";
  content: string;
}

export interface TranslatedBlock {
  x: number;
  y: number;
  w: number;
  h: number;
  lineHeight: number;
  lineCount: number;
  originalText: string;
  translatedText: string;
}

export interface BatchTextItem {
  index: number;
  title: string;
  sourceLang: string;
  originalText: string;
  translatedText: string;
  blocks?: TranslatedBlock[];
}

export interface StageRegionsOutcome {
  added: number;
  count: number;
  maxSize: number;
  latestThumb: string;
}

export interface RegionInput {
  rx: number;
  ry: number;
  rw: number;
  rh: number;
  annotatedBase64?: string | null;
  label?: string | null;
}

export interface OverlayThumb {
  index: number;
  label: string;
  imageDataUrl: string;
  pixelWidth: number;
  pixelHeight: number;
}

export type OverlayAnalysisState =
  | { status: "idle" }
  | {
      status: "ask";
      jobId: number;
      thumbs: OverlayThumb[];
      messages: AiChatMessage[];
      loading: boolean;
      error?: string | null;
    }
  | {
      status: "translate";
      jobId: number;
      thumbs: OverlayThumb[];
      engine: TranslateEngine;
      targetLang: string;
      loading: boolean;
      items: BatchTextItem[];
      elapsedMs: number;
      model: string;
      error?: string | null;
    }
  | {
      status: "ocr";
      jobId: number;
      thumbs: OverlayThumb[];
      loading: boolean;
      items: BatchTextItem[];
      elapsedMs: number;
      model: string;
      error?: string | null;
    };

export interface StackItemDto {
  id: number;
  title: string;
  thumbDataUrl: string;
  pixelWidth: number;
  pixelHeight: number;
}

export interface StackItemImageDto {
  id: number;
  title: string;
  imageDataUrl: string;
  pixelWidth: number;
  pixelHeight: number;
  scaleFactor: number;
}

export interface StackViewState {
  items: StackItemDto[];
  maxStackSize: number;
  clearAfterOutput: boolean;
  askJobId: number;
  askMessages: AiChatMessage[];
  askLoading: boolean;
  askError?: string | null;
  batchJobId: number;
  batchMode: string;
  batchLoading: boolean;
  batchError?: string | null;
  batchItems: BatchTextItem[];
  batchElapsedMs: number;
  landingCaptureId?: number;
  targetLang: string;
  engine: TranslateEngine;
}

export interface StreamPreviewEvent {
  jobId: number;
  kind: "ask" | "translate" | "ocr" | "text";
  text: string;
  originalText?: string | null;
  itemIndex?: number | null;
}

export interface PinItemDto {
  id: number;
  imageDataUrl: string;
  pixelWidth: number;
  pixelHeight: number;
  scaleFactor: number;
}

export interface TranslationResult {
  id: number;
  mode: "translate" | "ocr";
  engine: TranslateEngine;
  sourceLang: string;
  targetLang: string;
  originalText: string;
  translatedText: string;
  blocks?: TranslatedBlock[];
  elapsedMs: number;
  model: string;
  imageDataUrl: string;
  createdAtMs: number;
}

export type ResultViewState =
  | { status: "idle" }
  | {
      status: "loading";
      jobId: number;
      mode: "translate" | "ocr";
      engine: TranslateEngine;
      imageDataUrl: string;
      targetLang: string;
      pixelWidth: number;
      pixelHeight: number;
    }
  | {
      status: "done";
      result: TranslationResult;
      hasSourceImage: boolean;
    }
  | {
      status: "pin";
      id: number;
      imageDataUrl: string;
      pixelWidth: number;
      pixelHeight: number;
      scaleFactor: number;
    }
  | {
      status: "ask";
      jobId: number;
      imageDataUrl: string;
      pixelWidth: number;
      pixelHeight: number;
      messages: AiChatMessage[];
      loading: boolean;
      error?: string | null;
      sourceTextPreview?: string | null;
    }
  | {
      status: "error";
      jobId: number;
      mode: "translate" | "ocr";
      imageDataUrl: string;
      message: string;
    };

export interface ShortcutStatus {
  active: boolean;
  shortcut: string;
  error?: string | null;
  clipboardShortcut?: string;
  clipboardActive?: boolean;
  clipboardError?: string | null;
  selectionShortcut?: string;
  selectionActive?: boolean;
  selectionError?: string | null;
}

export interface ProviderPreset {
  id: string;
  label: string;
  baseUrl: string;
  defaultModel: string;
  hint: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "openai",
    label: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    defaultModel: "gpt-4o-mini",
    hint: "通用均衡，推荐 gpt-4o-mini / gpt-4.1-mini",
  },
  {
    id: "dashscope",
    label: "通义千问 (Qwen-VL)",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    defaultModel: "qwen-vl-plus",
    hint: "阿里云百炼视觉模型，支持识字与看图问答",
  },
  {
    id: "siliconflow",
    label: "硅基流动 (Qwen-VL)",
    baseUrl: "https://api.siliconflow.cn/v1",
    defaultModel: "Qwen/Qwen2.5-VL-72B-Instruct",
    hint: "国内直连速度快，OCR 准确率高",
  },
  {
    id: "zhipu",
    label: "智谱 (GLM-4V)",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    defaultModel: "glm-4v-flash",
    hint: "glm-4v-flash 免费/低成本且响应快",
  },
  {
    id: "gemini",
    label: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    defaultModel: "gemini-2.5-flash",
    hint: "OpenAI 兼容接口，多语言能力强",
  },
  {
    id: "ollama",
    label: "本地 Ollama",
    baseUrl: "http://127.0.0.1:11434/v1",
    defaultModel: "qwen2.5vl:7b",
    hint: "完全离线运行，API Key 可留空",
  },
];

export const TARGET_LANGUAGES = [
  "简体中文",
  "繁体中文",
  "English",
  "日本語",
  "한국어",
  "Français",
  "Deutsch",
  "Español",
];

export const SHORTCUT_PRESETS = ["Alt+Q", "Ctrl+Shift+A", "Ctrl+Alt+T", "F4"];

export const ENGINE_OPTIONS: { id: TranslateEngine; label: string; desc: string }[] = [
  { id: "ai", label: "视觉 AI", desc: "直接看图翻译与问答" },
  { id: "google", label: "Google", desc: "本地识字 + Google 翻译" },
  { id: "deepl", label: "DeepL", desc: "本地识字 + DeepL 翻译" },
];

let mermaidInitialized = false;
let katexModulePromise: Promise<typeof import("katex")> | null = null;
let mermaidModulePromise: Promise<typeof import("mermaid")> | null = null;

const loadKatexModule = () => {
  katexModulePromise ||= import("katex");
  return katexModulePromise;
};

const loadMermaidModule = () => {
  mermaidModulePromise ||= import("mermaid");
  return mermaidModulePromise;
};

export async function getKatexRenderer() {
  return (await loadKatexModule()).default;
}

export async function getMermaidRenderer() {
  const mermaid = (await loadMermaidModule()).default;
  if (!mermaidInitialized) {
    mermaid.initialize({
      startOnLoad: false,
      theme: "dark",
      securityLevel: "strict",
    });
    mermaidInitialized = true;
  }
  return mermaid;
}

export async function preloadRenderLibraries(): Promise<void> {
  await Promise.allSettled([loadKatexModule(), loadMermaidModule()]);

  if (document.fonts) {
    await Promise.allSettled([
      document.fonts.load("16px KaTeX_AMS"),
      document.fonts.load("16px KaTeX_Caligraphic"),
      document.fonts.load("16px KaTeX_Fraktur"),
      document.fonts.load("16px KaTeX_Main"),
      document.fonts.load("16px KaTeX_Math"),
      document.fonts.load("16px KaTeX_SansSerif"),
      document.fonts.load("16px KaTeX_Script"),
      document.fonts.load("16px KaTeX_Size1"),
      document.fonts.load("16px KaTeX_Size2"),
      document.fonts.load("16px KaTeX_Size3"),
      document.fonts.load("16px KaTeX_Size4"),
      document.fonts.load("16px KaTeX_Typewriter"),
    ]);
  }
}

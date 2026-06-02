import katex from "katex";
import mermaid from "mermaid";

let mermaidInitialized = false;

const MERMAID_PRELOAD_SAMPLES = [
  "flowchart TD\nA-->B",
  "sequenceDiagram\nAlice->>Bob: Hi",
  "classDiagram\nclass Animal",
  "stateDiagram-v2\n[*] --> Still",
  "erDiagram\nCUSTOMER ||--o{ ORDER : places",
  "gantt\ntitle Demo\ndateFormat YYYY-MM-DD\nsection Work\nTask :a1, 2024-01-01, 1d",
  'pie title Demo\n"Dogs" : 40\n"Cats" : 60',
  "gitGraph\ncommit",
  "journey\ntitle Day\nsection Work\nMake tea: 5: Me",
  "C4Context\ntitle Demo\nPerson(user, \"User\")\nSystem(system, \"System\")\nRel(user, system, \"Uses\")",
  "quadrantChart\ntitle Demo\nx-axis Low --> High\ny-axis Low --> High\nquadrant-1 Expand\nA: [0.3, 0.6]",
  'xychart-beta\ntitle "Sales"\nx-axis [Jan, Feb]\ny-axis "Revenue" 0 --> 100\nbar [10, 20]',
  "timeline\ntitle Demo\nsection Phase\n2024 : Event",
  "mindmap\n  root((Demo))\n    Branch",
  "sankey-beta\nA,B,1\nB,C,2",
  "block-beta\ncolumns 1\nA",
  "kanban\n  todo[Todo]\n    id1[Task]",
  "requirementDiagram\nrequirement test_req {\nid: 1\ntext: demo\nrisk: high\nverifymethod: test\n}",
  "architecture-beta\ngroup api(cloud)[API]\nservice server(server)[Server] in api",
  "packet-beta\n0-7: Byte",
  "radar-beta\naxis A\ncurve c{1}",
  "ishikawa-beta\nroot((Problem))\n  Cause",
  "venn-beta\nA, B: 1",
  "treemap\nRoot\n  Child: 1",
  "wardley-beta\ntitle Demo",
];

export function getKatexRenderer() {
  return katex;
}

export function getMermaidRenderer() {
  if (!mermaidInitialized) {
    mermaid.initialize({ startOnLoad: false, theme: "dark" });
    mermaidInitialized = true;
  }
  return mermaid;
}

export async function preloadRenderLibraries(): Promise<void> {
  const renderer = getMermaidRenderer();
  const mermaidResults = await Promise.allSettled(
    MERMAID_PRELOAD_SAMPLES.map((sample) => renderer.parse(sample, { suppressErrors: true })),
  );
  const failedMermaidSamples = mermaidResults.filter((result) => result.status === "rejected").length;
  if (failedMermaidSamples > 0) {
    console.warn(`[RenderLibraryPreloader] ${failedMermaidSamples} Mermaid diagram samples failed to preload`);
  }

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

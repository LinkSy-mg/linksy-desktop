// Linksy logo explorations. Single source for the showcase page and the SVG export.
// Every mark is drawn in a 100x100 viewBox and colored through classes:
//   .a primary, .b secondary, .c overlap/deep, .m muted primary, .sa/.sb strokes.
// uid keeps defs ids unique when the same mark appears several times on a page.
(function () {
  const marks = [
    {
      id: "linked-rings",
      title: "Linked Rings",
      subtitle: "Interlock · two devices, one bond",
      why: "Two thick rings pass over and under each other, so the bond reads as structural, not just overlapping. Rounded strokes keep it friendly; the tilt adds motion.",
      svg: (u) => `
  <defs>
    <mask id="ma-${u}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">
      <rect width="100" height="100" fill="#fff"/>
      <g clip-path="url(#cb-${u})"><circle cx="63" cy="50" r="20" fill="none" stroke="#000" stroke-width="14"/></g>
    </mask>
    <mask id="mb-${u}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">
      <rect width="100" height="100" fill="#fff"/>
      <g clip-path="url(#ct-${u})"><circle cx="37" cy="50" r="20" fill="none" stroke="#000" stroke-width="14"/></g>
    </mask>
    <clipPath id="ct-${u}"><circle cx="50" cy="34.8" r="13"/></clipPath>
    <clipPath id="cb-${u}"><circle cx="50" cy="65.2" r="13"/></clipPath>
  </defs>
  <g transform="rotate(-24 50 50)">
    <circle class="sb" cx="63" cy="50" r="20" stroke-width="8" mask="url(#mb-${u})"/>
    <circle class="sa" cx="37" cy="50" r="20" stroke-width="8" mask="url(#ma-${u})"/>
  </g>`,
    },
    {
      id: "relay-nodes",
      title: "Relay Nodes",
      subtitle: "Peer to peer · data in flight",
      why: "A large node and a small node, joined by two packets mid-transfer. It shows the core action directly, a file moving from one device to another, with no server in between.",
      svg: () => `
  <circle class="a" cx="30" cy="64" r="15"/>
  <circle class="a" cx="48.1" cy="51.1" r="3.6"/>
  <circle class="b" cx="56.0" cy="45.4" r="3.6"/>
  <circle class="b" cx="72" cy="34" r="10"/>`,
    },
    {
      id: "open-l",
      title: "Open L",
      subtitle: "Lettermark · the handoff",
      why: "An L whose foot has slid away from the stem, heading toward a waiting dot. The gap is the moment of sending. It holds up as a monogram at favicon size.",
      svg: () => `
  <rect class="a" x="22" y="16" width="16" height="64" rx="8"/>
  <rect class="a" x="46" y="64" width="36" height="16" rx="8"/>
  <circle class="b" cx="74" cy="30" r="8"/>`,
    },
    {
      id: "handshake",
      title: "Handshake",
      subtitle: "Discovery · two devices find each other",
      why: "Each device sends out one arc toward the other, and they meet in the middle. It reads as discovery and pairing on the local network, a two-way exchange instead of a one-way broadcast.",
      svg: () => `
  <circle class="a" cx="24" cy="76" r="10"/>
  <path class="sa" d="M24 52 A24 24 0 0 1 48 76" stroke-width="9" stroke-linecap="round"/>
  <path class="sb" d="M76 48 A24 24 0 0 1 52 24" stroke-width="9" stroke-linecap="round"/>
  <circle class="b" cx="76" cy="24" r="10"/>`,
    },
    {
      id: "pair-grid",
      title: "Pair Grid",
      subtitle: "Dot matrix · code and letter",
      why: "A 3×3 grid recalls the QR and short codes used for pairing. The lit cells spell an L that ends on a second color, the trusted device.",
      svg: () => `
  <rect class="a" x="15" y="15" width="20" height="20" rx="6.5"/>
  <rect class="m" x="40" y="15" width="20" height="20" rx="6.5"/>
  <rect class="m" x="65" y="15" width="20" height="20" rx="6.5"/>
  <rect class="a" x="15" y="40" width="20" height="20" rx="6.5"/>
  <rect class="m" x="40" y="40" width="20" height="20" rx="6.5"/>
  <rect class="m" x="65" y="40" width="20" height="20" rx="6.5"/>
  <rect class="a" x="15" y="65" width="20" height="20" rx="6.5"/>
  <rect class="a" x="40" y="65" width="20" height="20" rx="6.5"/>
  <rect class="b" x="65" y="65" width="20" height="20" rx="6.5"/>`,
    },
    {
      id: "loop",
      title: "Loop",
      subtitle: "Continuous · always in sync",
      why: "One unbroken stroke makes two lobes, two devices on a single path. The color shift across it suggests clipboard and files moving both ways.",
      svg: (u) => `
  <defs>
    <linearGradient id="lg-${u}" gradientUnits="userSpaceOnUse" x1="14" y1="50" x2="86" y2="50">
      <stop offset="0" style="stop-color:var(--a)"/>
      <stop offset="1" style="stop-color:var(--b)"/>
    </linearGradient>
  </defs>
  <path d="M50 50 C62 31 86 31 86 50 C86 69 62 69 50 50 C38 31 14 31 14 50 C14 69 38 69 50 50 Z"
        fill="none" stroke="url(#lg-${u})" stroke-width="10" stroke-linejoin="round"/>`,
    },
    {
      id: "shared-screens",
      title: "Shared Screens",
      subtitle: "Overlap · one workspace",
      why: "A portrait phone and a landscape laptop overlap, and the shared area takes a third tone. It shows the product promise of separate devices that work as one space.",
      svg: (u) => `
  <defs><clipPath id="cl-${u}"><rect x="38" y="18" width="46" height="36" rx="11"/></clipPath></defs>
  <rect class="b" x="38" y="18" width="46" height="36" rx="11"/>
  <rect class="a" x="16" y="30" width="34" height="50" rx="11"/>
  <rect class="c" x="16" y="30" width="34" height="50" rx="11" clip-path="url(#cl-${u})"/>`,
    },
  ];

  const palettes = [
    { id: "signal", name: "Signal", note: "Electric blue and cyan. Reads as wireless and trustworthy, and stays crisp on light and dark.", a: "#2563FF", b: "#22C3E6", c: "#1438B8" },
    { id: "aurora", name: "Aurora", note: "Violet and teal. Clearly different from the blue used by AirDrop and KDE Connect, with more energy.", a: "#7457F5", b: "#14BFA2", c: "#4A2FCF" },
    { id: "coral", name: "Coral", note: "Coral and amber. Warm and approachable, and nothing else in the category uses it.", a: "#FF5A52", b: "#FFAE3A", c: "#D1352F" },
    { id: "mint", name: "Mint & Ink", note: "Fresh green with a deep ink partner. Calm and practical, like a well-made tool.", a: "#12B282", b: "#1E2A3B", bDark: "#D5DEEA", c: "#0A7A5A" },
    { id: "mono", name: "Mono", note: "Ink only. The baseline every mark must pass before color is added.", a: "currentColor", b: "currentColor", c: "currentColor" },
  ];

  globalThis.LinksyMarks = { marks, palettes };
})();

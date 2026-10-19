// Round 2: variations on the Handshake mark.
// Same conventions as marks.js (100x100 viewBox, .a/.b/.c/.m/.sa/.sb classes, uid for defs).
(function () {
  const base = `
  <circle class="a" cx="24" cy="76" r="10"/>
  <path class="sa" d="M24 52 A24 24 0 0 1 48 76" stroke-width="9" stroke-linecap="round"/>
  <path class="sb" d="M76 48 A24 24 0 0 1 52 24" stroke-width="9" stroke-linecap="round"/>
  <circle class="b" cx="76" cy="24" r="10"/>`;

  const marks = [
    {
      id: "handshake",
      title: "Handshake",
      subtitle: "Reference · round 1 pick",
      why: "The original for comparison. Two devices, one arc each, meeting across a diagonal.",
      svg: () => base,
    },
    {
      id: "handshake-tight",
      title: "Tight",
      subtitle: "Closer arcs · heavier stroke",
      why: "Larger arcs and a slightly heavier stroke shrink the gap in the middle. The two signals nearly touch, which adds tension, and the mark looks more solid at 16 px.",
      svg: () => `
  <circle class="a" cx="23" cy="77" r="11"/>
  <path class="sa" d="M23 50 A27 27 0 0 1 50 77" stroke-width="10" stroke-linecap="round"/>
  <path class="sb" d="M77 50 A27 27 0 0 1 50 23" stroke-width="10" stroke-linecap="round"/>
  <circle class="b" cx="77" cy="23" r="11"/>`,
    },
    {
      id: "handshake-spark",
      title: "Spark",
      subtitle: "Center dot · the link itself",
      why: "A small dot in the deep tone sits where the arcs meet. It stands for the connection the two devices share and gives the mark a clear center.",
      svg: () => base + `
  <circle class="c" cx="50" cy="50" r="5"/>`,
    },
    {
      id: "handshake-gradient",
      title: "Blend",
      subtitle: "Gradient arcs · colors mix in transit",
      why: "Each arc shifts from its own device's color toward the other's, so the two colors meet in the middle. It's still flat at the ends and gets one subtle gradient.",
      svg: (u) => `
  <defs>
    <linearGradient id="hg-${u}" gradientUnits="userSpaceOnUse" x1="24" y1="76" x2="76" y2="24">
      <stop offset=".2" style="stop-color:var(--a)"/>
      <stop offset=".8" style="stop-color:var(--b)"/>
    </linearGradient>
  </defs>
  <circle class="a" cx="24" cy="76" r="10"/>
  <path d="M24 52 A24 24 0 0 1 48 76" fill="none" stroke="url(#hg-${u})" stroke-width="9" stroke-linecap="round"/>
  <path d="M76 48 A24 24 0 0 1 52 24" fill="none" stroke="url(#hg-${u})" stroke-width="9" stroke-linecap="round"/>
  <circle class="b" cx="76" cy="24" r="10"/>`,
    },
    {
      id: "handshake-asym",
      title: "Sender",
      subtitle: "Asymmetric · big to small",
      why: "The sending device is larger and reaches further, and the receiver answers with a smaller arc. The imbalance adds direction and movement, like a file going from laptop to phone.",
      svg: () => `
  <circle class="a" cx="25" cy="75" r="12"/>
  <path class="sa" d="M25 47 A28 28 0 0 1 53 75" stroke-width="9.5" stroke-linecap="round"/>
  <path class="sb" d="M78 42 A20 20 0 0 1 58 22" stroke-width="8" stroke-linecap="round"/>
  <circle class="b" cx="78" cy="22" r="8"/>`,
    },
    {
      id: "handshake-devices",
      title: "Devices",
      subtitle: "Rounded squares · screens, not points",
      why: "Swapping the dots for rounded squares turns them into screens. The mark gets closer to the product while staying geometric.",
      svg: () => `
  <rect class="a" x="13" y="66" width="21" height="21" rx="6.5"/>
  <path class="sa" d="M24 50 A26 26 0 0 1 50 76" stroke-width="9" stroke-linecap="round"/>
  <path class="sb" d="M76 50 A26 26 0 0 1 50 24" stroke-width="9" stroke-linecap="round"/>
  <rect class="b" x="66" y="13" width="21" height="21" rx="6.5"/>`,
    },
    {
      id: "handshake-level",
      title: "Level",
      subtitle: "Horizontal · built for the lockup",
      why: "The same mark turned 45° so the devices sit side by side, like two people talking. It lines up with the word linksy and fits wide spaces such as title bars.",
      svg: () => `
  <g transform="translate(50 50) rotate(45) scale(.92) translate(-50 -50)">${base}
  </g>`,
    },
    {
      id: "handshake-live",
      title: "Live",
      subtitle: "Animated state · searching for devices",
      why: "The static mark with its arcs pulsing in turn. It works as the in-app discovery indicator, so the logo itself shows that Linksy is looking for nearby devices.",
      animated: true,
      svg: () => `
  <circle class="a" cx="24" cy="76" r="10"/>
  <path class="sa pulse-1" d="M24 52 A24 24 0 0 1 48 76" stroke-width="9" stroke-linecap="round"/>
  <path class="sb pulse-2" d="M76 48 A24 24 0 0 1 52 24" stroke-width="9" stroke-linecap="round"/>
  <circle class="b" cx="76" cy="24" r="10"/>`,
    },
  ];

  globalThis.LinksyMarks = { marks, palettes: globalThis.LinksyMarks?.palettes ?? [] };
})();

import React from "react";
import {
  AbsoluteFill,
  Img,
  interpolate,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { loadFont as loadGrotesk } from "@remotion/google-fonts/SpaceGrotesk";
import { loadFont as loadInter } from "@remotion/google-fonts/Inter";
import { loadFont as loadMono } from "@remotion/google-fonts/JetBrainsMono";

const grotesk = loadGrotesk("normal", { weights: ["500", "700"], subsets: ["latin"] }).fontFamily;
const inter = loadInter("normal", { weights: ["400", "500", "600"], subsets: ["latin"] }).fontFamily;
const mono = loadMono("normal", { weights: ["400", "600"], subsets: ["latin"] }).fontFamily;

// Brand tokens (brand/BRAND.md)
const INK = "#0B0B0B";
const PAPER = "#FFFFFF";
const SIGNAL = "#FF5A1F";
const MIST = "#8A8F98";

// Scene timeline, in frames at 30 fps.
const T = {
  hook: [0, 100],
  better: [100, 160],
  logo: [160, 240],
  share: [240, 340],
  pair: [340, 430],
  browse: [430, 550],
  arrive: [550, 630],
  proof: [630, 750],
  trust: [750, 840],
  direction: [840, 900],
  end: [900, 1005],
} as const;
export const PROMO_FRAMES = T.end[1];

/* ---------- layout helpers ---------- */

// Scene-local frame. Scenes read it from context so the contact sheet can render any
// frame of the promo inside one still (Remotion's own Sequence follows the real timeline).
const FrameCtx = React.createContext<number | null>(null);
const useF = (): number => {
  const real = useCurrentFrame();
  const local = React.useContext(FrameCtx);
  return local ?? real;
};

// Lets the contact sheet render the promo at its real size inside small tiles.
export const SizeContext = React.createContext<{ width: number; height: number } | null>(null);

const useLayout = () => {
  const cfg = useVideoConfig();
  const ctx = React.useContext(SizeContext);
  const width = ctx?.width ?? cfg.width;
  const height = ctx?.height ?? cfg.height;
  const wide = width > height;
  // u = type/scale unit. 16:9 designed at 1920 wide; 1:1 a bit smaller.
  const u = wide ? width / 1920 : width / 1350;
  return { width, height, wide, u };
};

const enter = (frame: number, fps: number, delay = 0, damping = 18) =>
  spring({ frame: frame - delay, fps, config: { damping, stiffness: 140, mass: 0.8 } });

// Fake motion blur: blur proportional to how fast a spring is moving.
const speedBlur = (frame: number, fps: number, delay: number, k = 120) => {
  const v = Math.abs(enter(frame, fps, delay) - enter(frame - 1, fps, delay));
  return Math.min(14, Math.max(0, v * k - 0.8));
};

// Fade + blur out over the last `n` frames of a scene.
const useExit = (duration: number, n = 7) => {
  const frame = useF();
  const t = interpolate(frame, [duration - n, duration], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.in(Easing.cubic),
  });
  return { opacity: 1 - t, filter: `blur(${t * 12}px)`, transform: `scale(${1 + t * 0.04})` };
};

/* ---------- backgrounds ---------- */

const DarkBg: React.FC<{ children?: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill
    style={{
      background: `radial-gradient(1200px 800px at 75% 20%, #26262a 0%, rgba(11,11,11,0) 60%),
        radial-gradient(900px 700px at 10% 100%, #1c1c1f 0%, rgba(11,11,11,0) 60%), ${INK}`,
    }}
  >
    {children}
  </AbsoluteFill>
);

const LightBg: React.FC<{ children?: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill
    style={{
      background: `radial-gradient(1100px 800px at 85% 10%, #FFF1EA 0%, rgba(255,255,255,0) 55%),
        radial-gradient(1000px 800px at 0% 100%, #EEF0F3 0%, rgba(255,255,255,0) 60%), ${PAPER}`,
    }}
  >
    {children}
  </AbsoluteFill>
);

/* ---------- logo (bird cropped out of the padded square webp) ---------- */

const Logo: React.FC<{ w: number; dark?: boolean }> = ({ w, dark }) => {
  // Bird occupies roughly x 230..1040, y 290..845 of the 1254 px source.
  const s = w / 810;
  return (
    <div style={{ width: w, height: 555 * s, overflow: "hidden", position: "relative" }}>
      <Img
        src={staticFile("logo.webp")}
        style={{
          position: "absolute",
          width: 1254 * s,
          left: -230 * s,
          top: -290 * s,
          filter: dark ? "invert(1)" : undefined,
          mixBlendMode: dark ? "screen" : "multiply",
        }}
      />
    </div>
  );
};

/* ---------- kinetic words ---------- */

type Word = string | { text: string; hi?: boolean; chip?: React.ReactNode };

const Words: React.FC<{
  words: Word[];
  size: number;
  color?: string;
  delay?: number;
  stagger?: number;
  align?: "left" | "center";
  weight?: number;
  maxWidth?: number;
}> = ({ words, size, color = PAPER, delay = 0, stagger = 4, align = "left", weight = 700, maxWidth }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        justifyContent: align === "center" ? "center" : "flex-start",
        alignItems: "center",
        columnGap: size * 0.26,
        rowGap: size * 0.08,
        fontFamily: grotesk,
        fontWeight: weight,
        fontSize: size,
        lineHeight: 1.08,
        letterSpacing: -size * 0.025,
        color,
        maxWidth,
      }}
    >
      {words.map((w, i) => {
        const d = delay + i * stagger;
        const p = enter(frame, fps, d);
        const blur = speedBlur(frame, fps, d, 90);
        const word = typeof w === "string" ? { text: w } : w;
        const hiP = word.hi ? interpolate(frame, [d + 8, d + 18], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 0;
        return (
          <span
            key={i}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: size * 0.18,
              opacity: p,
              transform: `translateY(${(1 - p) * size * 0.6}px)`,
              filter: `blur(${blur}px)`,
              position: "relative",
            }}
          >
            {word.chip}
            <span style={{ position: "relative", color: word.hi ? (hiP > 0.5 ? SIGNAL : color) : color }}>
              {word.text}
              {word.hi ? (
                <span
                  style={{
                    position: "absolute",
                    left: 0,
                    bottom: -size * 0.06,
                    height: size * 0.07,
                    width: `${hiP * 100}%`,
                    background: SIGNAL,
                    borderRadius: size,
                  }}
                />
              ) : null}
            </span>
          </span>
        );
      })}
    </div>
  );
};

const Chip: React.FC<{ size: number; dark?: boolean; children: React.ReactNode }> = ({ size, dark, children }) => (
  <span
    style={{
      display: "inline-flex",
      alignItems: "center",
      justifyContent: "center",
      width: size * 1.15,
      height: size * 0.9,
      borderRadius: size * 0.24,
      background: dark ? "#1E1E21" : "#F1F1F2",
      border: `1px solid ${dark ? "#33333a" : "#E2E2E5"}`,
      boxShadow: dark ? "0 8px 30px rgba(0,0,0,0.5)" : "0 8px 24px rgba(11,11,11,0.08)",
    }}
  >
    {children}
  </span>
);

const FileIcon: React.FC<{ s: number; color: string }> = ({ s, color }) => (
  <svg width={s} height={s} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={1.8} strokeLinejoin="round">
    <path d="M6 2.8h8l4.2 4.2V21.2H6z" />
    <path d="M14 2.8V7h4.2" />
  </svg>
);

const LockIcon: React.FC<{ s: number; color: string }> = ({ s, color }) => (
  <svg width={s} height={s} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={1.8} strokeLinecap="round">
    <rect x="4.5" y="10.5" width="15" height="10" rx="2" />
    <path d="M8 10.5V7.5a4 4 0 0 1 8 0v3" />
  </svg>
);

const FolderIcon: React.FC<{ s: number; color: string }> = ({ s, color }) => (
  <svg width={s} height={s} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={1.8} strokeLinejoin="round">
    <path d="M3 6.5h6.5l2 2.2H21V19H3z" />
  </svg>
);

/* ---------- real UI crops (app/design/screenshots on origin/app-design) ---------- */

type Crop = { src: string; x: number; y: number; w: number; h: number };
const UI = {
  popover: { src: "ui/mac-popover-light.png", x: 690, y: 36, w: 339, h: 417 },
  invite: { src: "ui/mac-invite-dark.png", x: 331, y: 98, w: 519, h: 302 },
  browse: { src: "ui/win-browse-light.png", x: 31, y: 1, w: 1039, h: 635 },
  browseDark: { src: "ui/win-browse-dark.png", x: 31, y: 1, w: 1039, h: 635 },
  finished: { src: "ui/win-transfers-dark.png", x: 264, y: 316, w: 806, h: 178 },
} satisfies Record<string, Crop>;

const Shot: React.FC<{ crop: Crop; w: number; radius?: number; dark?: boolean; children?: React.ReactNode; style?: React.CSSProperties }> = ({
  crop,
  w,
  radius = 14,
  dark,
  children,
  style,
}) => {
  const s = w / crop.w;
  return (
    <div
      style={{
        width: w,
        height: crop.h * s,
        position: "relative",
        overflow: "hidden",
        borderRadius: radius,
        boxShadow: dark
          ? "0 40px 120px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.08)"
          : "0 40px 100px rgba(11,11,11,0.16), 0 0 0 1px rgba(11,11,11,0.06)",
        ...style,
      }}
    >
      <Img
        src={staticFile(crop.src)}
        style={{ position: "absolute", left: -crop.x * s, top: -crop.y * s, width: (crop.src.startsWith("ui/mac") ? 1180 : 1100) * s }}
      />
      {children}
    </div>
  );
};

// Ring that marks a region of a crop (in crop pixels).
const Ring: React.FC<{ s: number; x: number; y: number; w: number; h: number; p: number; r?: number }> = ({ s, x, y, w, h, p, r = 8 }) => (
  <div
    style={{
      position: "absolute",
      left: (x - 4) * s,
      top: (y - 4) * s,
      width: (w + 8) * s,
      height: (h + 8) * s,
      borderRadius: r * s,
      border: `${3 * Math.max(1, s * 0.8)}px solid ${SIGNAL}`,
      opacity: p,
      transform: `scale(${1.15 - 0.15 * p})`,
    }}
  />
);

const PreviewTag: React.FC<{ dark?: boolean }> = ({ dark }) => {
  const { u, wide } = useLayout();
  return (
    <div
      style={{
        position: "absolute",
        left: wide ? 80 * u : 60 * u,
        bottom: 52 * u,
        fontFamily: mono,
        fontSize: 22 * u,
        color: dark ? "#6b7079" : MIST,
        letterSpacing: 0.5,
      }}
    >
      App design preview · not released yet
    </div>
  );
};

const StepLabel: React.FC<{ n: string; text: string; dark?: boolean }> = ({ n, text, dark }) => {
  const { u } = useLayout();
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 14 * u, fontFamily: mono, fontSize: 26 * u, color: MIST, marginBottom: 22 * u }}>
      <span
        style={{
          width: 40 * u,
          height: 40 * u,
          borderRadius: 40 * u,
          border: `2px solid ${dark ? "#3a3a40" : "#dcdce0"}`,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          color: dark ? PAPER : INK,
          fontSize: 22 * u,
        }}
      >
        {n}
      </span>
      {text}
    </div>
  );
};

/* ---------- scenes ---------- */

const BUBBLES = [
  { name: "report-q4.pdf", size: "2.4 MB", time: "08:52" },
  { name: "notes.md", size: "6 KB", time: "08:53" },
  { name: "report-q4-final.pdf", size: "2.4 MB", time: "09:10" },
  { name: "figures.zip", size: "41 MB", time: "09:11" },
  { name: "report-q4-final-v2.pdf", size: "2.5 MB", time: "09:12" },
  { name: "report-q4-FINAL.pdf", size: "2.5 MB", time: "09:40" },
];

const Bubble: React.FC<{ name: string; size: string; time: string; u: number }> = ({ name, size, time, u }) => (
  <div
    style={{
      display: "flex",
      alignItems: "center",
      gap: 18 * u,
      padding: `${16 * u}px ${22 * u}px`,
      background: "#232327",
      border: "1px solid #3a3a40",
      borderRadius: `${22 * u}px ${22 * u}px ${6 * u}px ${22 * u}px`,
      boxShadow: "0 20px 50px rgba(0,0,0,0.45)",
      width: 520 * u,
    }}
  >
    <div
      style={{
        width: 58 * u,
        height: 58 * u,
        borderRadius: 14 * u,
        background: "#2A2A2F",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
      }}
    >
      <FileIcon s={30 * u} color="#c9cbd1" />
    </div>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontFamily: inter, fontWeight: 600, fontSize: 25 * u, color: PAPER, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
        {name}
      </div>
      <div style={{ fontFamily: mono, fontSize: 19 * u, color: MIST, marginTop: 4 * u }}>
        {size} · to: me
      </div>
    </div>
    <div style={{ fontFamily: mono, fontSize: 18 * u, color: "#6b7079", alignSelf: "flex-end" }}>{time} ✓✓</div>
  </div>
);

const HookScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide } = useLayout();
  const exit = useExit(duration);
  const size = (wide ? 118 : 96) * u;
  const stack = (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 14 * u, transform: `rotate(-4deg)` }}>
      {BUBBLES.map((b, i) => {
        const d = 6 + i * 9;
        const p = enter(frame, fps, d, 15);
        return (
          <div
            key={i}
            style={{
              opacity: p,
              transform: `translateY(${(1 - p) * 80 * u}px) scale(${0.9 + 0.1 * p})`,
              filter: `blur(${speedBlur(frame, fps, d, 80)}px)`,
              marginRight: (i % 2) * 40 * u,
            }}
          >
            <Bubble {...b} u={wide ? u * 1.25 : u * 0.95} />
          </div>
        );
      })}
    </div>
  );
  return (
    <DarkBg>
      <AbsoluteFill style={{ ...exit, padding: wide ? `0 ${110 * u}px` : `${90 * u}px ${70 * u}px`, display: "flex", flexDirection: wide ? "row" : "column", alignItems: "center", justifyContent: wide ? "space-between" : "flex-start", gap: 40 * u }}>
        <div style={{ maxWidth: wide ? 900 * u : undefined }}>
          <Words
            size={size}
            delay={4}
            stagger={5}
            words={[
              "Sending",
              { text: "files", chip: <Chip size={size} dark><FileIcon s={size * 0.52} color={PAPER} /></Chip> },
              "to",
              "yourself",
              "on",
              "Telegram?",
            ]}
          />
        </div>
        <div style={{ marginTop: wide ? 0 : 30 * u }}>{stack}</div>
      </AbsoluteFill>
    </DarkBg>
  );
};

const BetterScene: React.FC<{ duration: number }> = ({ duration }) => {
  const { u, wide } = useLayout();
  const exit = useExit(duration, 6);
  return (
    <DarkBg>
      <AbsoluteFill style={{ ...exit, alignItems: "center", justifyContent: "center", padding: 80 * u }}>
        <Words size={(wide ? 132 : 112) * u} align="center" stagger={4} delay={2} words={["There's", "a", "better", "way", { text: "home.", hi: true }]} />
      </AbsoluteFill>
    </DarkBg>
  );
};

const LogoScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u } = useLayout();
  const exit = useExit(duration);
  const p = enter(frame, fps, 0, 14);
  const t = enter(frame, fps, 14);
  const g = enter(frame, fps, 24);
  return (
    <DarkBg>
      <AbsoluteFill style={{ ...exit, alignItems: "center", justifyContent: "center", flexDirection: "column" }}>
        <div style={{ transform: `translateX(${(1 - p) * -260 * u}px) translateY(${(1 - p) * 60 * u}px) rotate(${(1 - p) * -12}deg)`, opacity: p, filter: `blur(${speedBlur(frame, fps, 0, 140)}px)` }}>
          <Logo w={250 * u} dark />
        </div>
        <div style={{ fontFamily: grotesk, fontWeight: 700, fontSize: 150 * u, color: PAPER, letterSpacing: -4 * u, marginTop: 26 * u, opacity: t, transform: `translateY(${(1 - t) * 40 * u}px)` }}>
          Pigeon
        </div>
        <div style={{ fontFamily: inter, fontWeight: 500, fontSize: 46 * u, color: "#c9cbd1", marginTop: 8 * u, opacity: g, transform: `translateY(${(1 - g) * 30 * u}px)` }}>
          Your files know the way home.
        </div>
      </AbsoluteFill>
    </DarkBg>
  );
};

// Shared layout for the three "how it works" steps: headline + real UI card.
const StepLayout: React.FC<{
  duration: number;
  dark?: boolean;
  label: [string, string];
  words: Word[];
  card: React.ReactNode;
  cardDelay?: number;
  sub?: React.ReactNode;
  stacked?: boolean; // headline above the card, also in 16:9
}> = ({ duration, dark, label, words, card, cardDelay = 6, sub, stacked }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide: isWide } = useLayout();
  const wide = isWide && !stacked;
  const exit = useExit(duration);
  const p = enter(frame, fps, cardDelay, 20);
  const blur = speedBlur(frame, fps, cardDelay, 70);
  const Bg = dark ? DarkBg : LightBg;
  const drift = interpolate(frame, [0, duration], [0, -14 * u]);
  return (
    <Bg>
      <AbsoluteFill
        style={{
          ...exit,
          display: "flex",
          flexDirection: wide ? "row" : "column",
          alignItems: "center",
          justifyContent: wide ? "space-between" : "flex-start",
          padding: wide ? `0 ${100 * u}px 0 ${110 * u}px` : `${(isWide ? 64 : 80) * u}px ${60 * u}px`,
          gap: wide ? 60 * u : (isWide ? 36 : 50) * u,
        }}
      >
        <div style={{ width: wide ? 640 * u : "100%", flexShrink: 0, maxWidth: isWide && stacked ? 1240 * u : undefined }}>
          <StepLabel n={label[0]} text={label[1]} dark={dark} />
          <Words size={(wide ? 82 : 76) * u} color={dark ? PAPER : INK} delay={2} stagger={4} words={words} />
          {sub ? <div style={{ marginTop: 26 * u, opacity: enter(frame, fps, 20) }}>{sub}</div> : null}
        </div>
        <div
          style={{
            flex: 1,
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            opacity: p,
            transform: `translateX(${wide ? (1 - p) * 260 * u : 0}px) translateY(${(wide ? 0 : (1 - p) * 200 * u) + drift}px) rotateY(${(1 - p) * -14}deg) scale(${0.92 + 0.08 * p})`,
            filter: `blur(${blur}px)`,
            perspective: 1600,
          }}
        >
          {card}
        </div>
      </AbsoluteFill>
      <PreviewTag dark={dark} />
    </Bg>
  );
};

const subText = (u: number, dark?: boolean): React.CSSProperties => ({
  fontFamily: inter,
  fontSize: 32 * u,
  lineHeight: 1.35,
  color: dark ? "#b4b7be" : "#55595f",
});

const ShareScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { u, wide } = useLayout();
  const w = (wide ? 520 : 560) * u;
  const s = w / UI.popover.w;
  const ring = interpolate(frame, [34, 46], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <StepLayout
      duration={duration}
      label={["1", "On your work Mac"]}
      words={["Share", "a", { text: "folder.", chip: <Chip size={82 * u}><FolderIcon s={44 * u} color={INK} /></Chip> }, { text: "Read‑only.", hi: true }]}
      sub={<div style={subText(u)}>Only the folders you pick. Files stay on the Mac.</div>}
      card={
        <Shot crop={UI.popover} w={w} radius={18 * u}>
          {/* "SHARING · READ-ONLY" section: x 690..1029, y 120..220 in the source */}
          <Ring s={s} x={8} y={88} w={322} h={96} p={ring} r={10} />
        </Shot>
      }
    />
  );
};

const PairScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { u, wide } = useLayout();
  const w = (wide ? 940 : 900) * u;
  const s = w / UI.invite.w;
  const ring = interpolate(frame, [30, 42], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <StepLayout
      duration={duration}
      dark
      label={["2", "Pair once"]}
      words={["One", { text: "4‑word", hi: true }, "code.", "That's", "it."]}
      sub={<div style={subText(u, true)}>Typed on your home PC. Single use, never sent over the network.</div>}
      card={
        <Shot crop={UI.invite} w={w} radius={18 * u} dark>
          {/* code tiles: source x 383..827, y 295..360 */}
          <Ring s={s} x={52} y={197} w={444} h={65} p={ring} r={10} />
        </Shot>
      }
    />
  );
};

const Cursor: React.FC<{ size: number; press: number }> = ({ size, press }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" style={{ transform: `scale(${1 - press * 0.18})`, transformOrigin: "10% 10%", filter: "drop-shadow(0 4px 8px rgba(0,0,0,0.3))" }}>
    <path d="M4 2.5 L4 19 L8.4 15 L11.2 21.3 L14 20.1 L11.3 13.9 L17.3 13.6 Z" fill={INK} stroke={PAPER} strokeWidth={1.4} strokeLinejoin="round" />
  </svg>
);

const BrowseScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide } = useLayout();
  const w = (wide ? 1240 : 960) * u;
  const s = w / UI.browse.w;
  // Get button centre in crop pixels.
  const gx = 973;
  const gy = 152;
  const clickAt = 62;
  const move = interpolate(frame, [22, clickAt - 4], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.inOut(Easing.cubic) });
  const cx = interpolate(move, [0, 1], [560, gx - 6]);
  const cy = interpolate(move, [0, 1], [560, gy - 4]);
  const press = interpolate(frame, [clickAt - 3, clickAt, clickAt + 4], [0, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const ripple = interpolate(frame, [clickAt, clickAt + 16], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const zoom = 1 + 0.3 * enter(frame, fps, 30, 22) - 0.3 * enter(frame, fps, clickAt + 24, 22);
  const getP = interpolate(frame, [clickAt, clickAt + 6], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <StepLayout
      duration={duration}
      label={["3", "At home, on Windows"]}
      stacked
      words={["Browse", "the", "Mac.", "Click", { text: "Get.", hi: true }]}
      card={
        <div style={{ transform: `scale(${zoom})`, transformOrigin: `${(gx / UI.browse.w) * 100}% ${(gy / UI.browse.h) * 100}%` }}>
          <Shot crop={UI.browse} w={w} radius={14 * u}>
            {/* pressed state on the Get button: source x 973..1035, y 141..167 */}
            <div
              style={{
                position: "absolute",
                left: (942 - 1) * s,
                top: (140 - 1) * s,
                width: 64 * s,
                height: 28 * s,
                borderRadius: 6 * s,
                background: SIGNAL,
                opacity: getP,
                color: PAPER,
                fontFamily: inter,
                fontWeight: 600,
                fontSize: 13 * s,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 5 * s,
              }}
            >
              ↓ Get
            </div>
            <div
              style={{
                position: "absolute",
                left: gx * s - 40 * s,
                top: gy * s - 40 * s,
                width: 80 * s,
                height: 80 * s,
                borderRadius: 80 * s,
                border: `${3 * s}px solid ${SIGNAL}`,
                opacity: ripple > 0 && ripple < 1 ? 1 - ripple : 0,
                transform: `scale(${0.3 + ripple})`,
              }}
            />
            <div style={{ position: "absolute", left: cx * s, top: cy * s }}>
              <Cursor size={34 * s} press={press} />
            </div>
          </Shot>
        </div>
      }
    />
  );
};

const ArriveScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { u, wide } = useLayout();
  const w = (wide ? 1000 : 960) * u;
  const s = w / UI.finished.w;
  const ring = interpolate(frame, [26, 38], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <StepLayout
      duration={duration}
      label={["4", "On your home PC"]}
      words={["It", "arrives.", { text: "Verified.", hi: true }]}
      sub={<div style={subText(u)}>Every file is checked with SHA‑256. Nothing is overwritten.</div>}
      card={
        <Shot crop={UI.finished} w={w} radius={16 * u} dark>
          {/* "SHA-256 verified" + "Direct": source x 334..508, y 372..392 */}
          <Ring s={s} x={70} y={56} w={176} h={22} p={ring} r={6} />
        </Shot>
      }
    />
  );
};

const ProofScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide } = useLayout();
  const exit = useExit(duration);
  // Day 4 (DEVLOG.md on origin/day-4-office-test): 50 MiB in 2.44 s, 21.5 MB/s, direct, SHA-256 match.
  const secs = interpolate(frame, [16, 58], [0, 2.44], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.out(Easing.cubic) });
  const done = frame >= 58;
  const a = enter(frame, fps, 2);
  const chips = ["21.5 MB/s", "Direct", "SHA‑256 match"];
  const big = (wide ? 190 : 150) * u;
  return (
    <DarkBg>
      <AbsoluteFill style={{ ...exit, alignItems: "center", justifyContent: "center", flexDirection: "column", padding: 60 * u }}>
        <div style={{ fontFamily: mono, fontSize: 28 * u, color: MIST, letterSpacing: 2, opacity: a, marginBottom: 30 * u, textAlign: "center" }}>
          REAL TEST · OFFICE MAC → HOME WINDOWS PC
        </div>
        <div style={{ display: "flex", alignItems: "baseline", gap: 34 * u, opacity: a, transform: `translateY(${(1 - a) * 40 * u}px)`, flexWrap: wide ? "nowrap" : "wrap", justifyContent: "center" }}>
          <span style={{ fontFamily: mono, fontWeight: 600, fontSize: big, color: PAPER, letterSpacing: -4 * u }}>50 MiB</span>
          <span style={{ fontFamily: grotesk, fontWeight: 500, fontSize: big * 0.42, color: MIST }}>in</span>
          <span style={{ fontFamily: mono, fontWeight: 600, fontSize: big, color: done ? SIGNAL : PAPER, letterSpacing: -4 * u, minWidth: big * 2.7, textAlign: "left" }}>
            {secs.toFixed(2)} s
          </span>
        </div>
        <div style={{ display: "flex", gap: 18 * u, marginTop: 50 * u, flexWrap: "wrap", justifyContent: "center" }}>
          {chips.map((c, i) => {
            const p = enter(frame, fps, 60 + i * 6);
            return (
              <div
                key={c}
                style={{
                  fontFamily: mono,
                  fontSize: 38 * u,
                  color: PAPER,
                  padding: `${14 * u}px ${28 * u}px`,
                  borderRadius: 999,
                  border: "1.5px solid #3a3a40",
                  background: "#18181b",
                  opacity: p,
                  transform: `translateY(${(1 - p) * 30 * u}px)`,
                  filter: `blur(${speedBlur(frame, fps, 60 + i * 6, 80)}px)`,
                }}
              >
                {i === 2 ? "✓ " : ""}
                {c}
              </div>
            );
          })}
        </div>
        <div style={{ fontFamily: inter, fontSize: 26 * u, color: "#6b7079", marginTop: 46 * u, opacity: enter(frame, fps, 76), textAlign: "center" }}>
          Two different networks · measured 23 Nov 2026 · CLI build
        </div>
      </AbsoluteFill>
    </DarkBg>
  );
};

const TrustScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide } = useLayout();
  const exit = useExit(duration);
  const lines = ["No cloud copy.", "No VPN.", "No open ports."];
  const size = (wide ? 120 : 100) * u;
  const e2eAt = 50;
  return (
    <LightBg>
      <AbsoluteFill style={{ ...exit, alignItems: wide ? "flex-start" : "center", justifyContent: "center", flexDirection: "column", padding: `0 ${150 * u}px` }}>
        {lines.map((l, i) => {
          const d = i * 12;
          const p = enter(frame, fps, d);
          return (
            <div
              key={l}
              style={{
                fontFamily: grotesk,
                fontWeight: 700,
                fontSize: size,
                letterSpacing: -size * 0.025,
                lineHeight: 1.12,
                color: INK,
                opacity: p,
                transform: `translateX(${(1 - p) * -80 * u}px)`,
                filter: `blur(${speedBlur(frame, fps, d, 90)}px)`,
              }}
            >
              {l}
            </div>
          );
        })}
        <div style={{ marginTop: 40 * u, display: "flex", alignItems: "center", gap: 18 * u, opacity: enter(frame, fps, e2eAt), transform: `translateY(${(1 - enter(frame, fps, e2eAt)) * 30 * u}px)` }}>
          <Chip size={70 * u}>
            <LockIcon s={38 * u} color={SIGNAL} />
          </Chip>
          <span style={{ fontFamily: inter, fontWeight: 600, fontSize: 50 * u, color: INK }}>End‑to‑end encrypted.</span>
        </div>
      </AbsoluteFill>
    </LightBg>
  );
};

const DirectionScene: React.FC<{ duration: number }> = ({ duration }) => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide } = useLayout();
  const exit = useExit(duration, 6);
  const a = enter(frame, fps, 0);
  const arrow = interpolate(frame, [6, 20], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const size = (wide ? 140 : 110) * u;
  return (
    <LightBg>
      <AbsoluteFill style={{ ...exit, alignItems: "center", justifyContent: "center", flexDirection: "column" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 40 * u, fontFamily: grotesk, fontWeight: 700, fontSize: size, color: INK, letterSpacing: -size * 0.025, opacity: a }}>
          <span style={{ transform: `translateX(${(1 - a) * -60 * u}px)` }}>Mac</span>
          <span style={{ color: SIGNAL, fontFamily: mono, fontWeight: 400, transform: `scaleX(${arrow})` }}>⇄</span>
          <span style={{ transform: `translateX(${(1 - a) * 60 * u}px)` }}>Windows</span>
        </div>
        <div style={{ fontFamily: inter, fontWeight: 500, fontSize: 40 * u, color: "#55595f", marginTop: 20 * u, opacity: enter(frame, fps, 14) }}>
          Any direction. Linux coming soon.
        </div>
      </AbsoluteFill>
    </LightBg>
  );
};

const EndScene: React.FC = () => {
  const frame = useF();
  const { fps } = useVideoConfig();
  const { u, wide, width } = useLayout();
  const bg = enter(frame, fps, 0, 26);
  const a = enter(frame, fps, 4);
  const b = enter(frame, fps, 12);
  const c = enter(frame, fps, 20);
  const bgW = (wide ? 1700 : 1500) * u;
  return (
    <DarkBg>
      {/* the real UI, floating behind the end card */}
      <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", opacity: 0.38 * bg }}>
        <div style={{ transform: `scale(${1.08 - 0.08 * bg}) rotate(-6deg) translateY(${interpolate(frame, [0, 105], [0, -30]) * u}px)`, filter: "blur(6px)" }}>
          <Shot crop={UI.browseDark} w={Math.max(bgW, width * 0.9)} dark />
        </div>
      </AbsoluteFill>
      <AbsoluteFill style={{ background: "radial-gradient(closest-side, rgba(11,11,11,0.85), rgba(11,11,11,0.35))" }} />
      <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", flexDirection: "column" }}>
        <div style={{ opacity: a, transform: `translateY(${(1 - a) * 40 * u}px)`, display: "flex", alignItems: "center", gap: 30 * u }}>
          <Logo w={170 * u} dark />
          <span style={{ fontFamily: grotesk, fontWeight: 700, fontSize: 130 * u, color: PAPER, letterSpacing: -4 * u }}>Pigeon</span>
        </div>
        <div style={{ fontFamily: inter, fontWeight: 500, fontSize: 42 * u, color: "#c9cbd1", marginTop: 14 * u, opacity: a }}>
          Your files know the way home.
        </div>
        <div
          style={{
            marginTop: 54 * u,
            opacity: b,
            transform: `scale(${0.9 + 0.1 * b})`,
            fontFamily: grotesk,
            fontWeight: 700,
            fontSize: 54 * u,
            color: PAPER,
            background: SIGNAL,
            padding: `${16 * u}px ${44 * u}px`,
            borderRadius: 999,
          }}
        >
          Coming soon
        </div>
        <div style={{ marginTop: 40 * u, fontFamily: mono, fontSize: 40 * u, color: PAPER, opacity: c }}>flypigeon.pages.dev</div>
        <div style={{ marginTop: 14 * u, fontFamily: inter, fontSize: 30 * u, color: MIST, opacity: c }}>Built in public by @aliinprod</div>
      </AbsoluteFill>
    </DarkBg>
  );
};

/* ---------- the cut ---------- */

const len = (k: keyof typeof T) => T[k][1] - T[k][0];

const SCENES: [keyof typeof T, React.ReactNode][] = [
  ["hook", <HookScene duration={len("hook")} />],
  ["better", <BetterScene duration={len("better")} />],
  ["logo", <LogoScene duration={len("logo")} />],
  ["share", <ShareScene duration={len("share")} />],
  ["pair", <PairScene duration={len("pair")} />],
  ["browse", <BrowseScene duration={len("browse")} />],
  ["arrive", <ArriveScene duration={len("arrive")} />],
  ["proof", <ProofScene duration={len("proof")} />],
  ["trust", <TrustScene duration={len("trust")} />],
  ["direction", <DirectionScene duration={len("direction")} />],
  ["end", <EndScene />],
];

// `at` renders one fixed frame (used by the contact sheet); otherwise follows the timeline.
export const Promo: React.FC<{ at?: number }> = ({ at }) => {
  const real = useCurrentFrame();
  const g = at ?? real;
  const current = SCENES.find(([k]) => g >= T[k][0] && g < T[k][1]);
  return (
    <AbsoluteFill style={{ background: INK }}>
      {current ? (
        <FrameCtx.Provider key={current[0]} value={g - T[current[0]][0]}>
          {current[1]}
        </FrameCtx.Provider>
      ) : null}
    </AbsoluteFill>
  );
};

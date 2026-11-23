import React from "react";
import { AbsoluteFill } from "remotion";
import { Promo, PROMO_FRAMES, SizeContext } from "./Promo";

// One frame every 2 s (t = 1 s, 3 s, 5 s, ...), rendered live from the promo itself.
export const SHEET_COLS = 4;
const FPS = 30;
export const sheetFrames = (): number[] => {
  const out: number[] = [];
  for (let f = FPS; f < PROMO_FRAMES; f += 2 * FPS) out.push(f);
  return out;
};

export const sheetSize = (w: number, h: number) => {
  const tileW = 480;
  const tileH = (tileW * h) / w;
  const gap = 8;
  const label = 28;
  const rows = Math.ceil(sheetFrames().length / SHEET_COLS);
  return {
    tileW,
    tileH,
    gap,
    label,
    width: SHEET_COLS * tileW + (SHEET_COLS + 1) * gap,
    height: rows * (tileH + label) + (rows + 1) * gap,
  };
};

export const ContactSheet: React.FC<{ w: number; h: number }> = ({ w, h }) => {
  const s = sheetSize(w, h);
  return (
    <AbsoluteFill style={{ background: "#0B0B0B" }}>
      <SizeContext.Provider value={{ width: w, height: h }}>
        {sheetFrames().map((f, i) => {
          const col = i % SHEET_COLS;
          const row = Math.floor(i / SHEET_COLS);
          return (
            <div
              key={f}
              style={{
                position: "absolute",
                left: s.gap + col * (s.tileW + s.gap),
                top: s.gap + row * (s.tileH + s.label + s.gap),
                width: s.tileW,
                height: s.tileH + s.label,
              }}
            >
              <div style={{ height: s.label, color: "#8A8F98", fontFamily: "monospace", fontSize: 18 }}>
                {(f / FPS).toFixed(0)} s · frame {f}
              </div>
              <div style={{ width: s.tileW, height: s.tileH, overflow: "hidden", position: "relative" }}>
                <div style={{ width: w, height: h, transform: `scale(${s.tileW / w})`, transformOrigin: "0 0", position: "absolute" }}>
                  <Promo at={f} />
                </div>
              </div>
            </div>
          );
        })}
      </SizeContext.Provider>
    </AbsoluteFill>
  );
};

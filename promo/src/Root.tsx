import React from "react";
import { Composition } from "remotion";
import { Promo, PROMO_FRAMES } from "./Promo";
import { ContactSheet, sheetSize } from "./ContactSheet";

const sheet169 = sheetSize(1920, 1080);
const sheet11 = sheetSize(1080, 1080);

export const Root: React.FC = () => (
  <>
    <Composition id="Promo16x9" component={Promo} durationInFrames={PROMO_FRAMES} fps={30} width={1920} height={1080} />
    <Composition id="Promo1x1" component={Promo} durationInFrames={PROMO_FRAMES} fps={30} width={1080} height={1080} />
    <Composition
      id="Contact16x9"
      component={ContactSheet}
      defaultProps={{ w: 1920, h: 1080 }}
      durationInFrames={1}
      fps={30}
      width={sheet169.width}
      height={Math.ceil(sheet169.height)}
    />
    <Composition
      id="Contact1x1"
      component={ContactSheet}
      defaultProps={{ w: 1080, h: 1080 }}
      durationInFrames={1}
      fps={30}
      width={sheet11.width}
      height={Math.ceil(sheet11.height)}
    />
  </>
);

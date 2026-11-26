import { describe, expect, it } from 'vitest';

import { modelMenuPosition } from './ai-model-menu-position';

describe('model menu placement', () => {
  it('expands outside a narrow panel without crossing the right window edge', () => {
    const style = modelMenuPosition({ left: 1000, top: 680, bottom: 710 }, 1280, 800);
    expect(style).toMatchObject({ width: '460px', left: '812px', bottom: '128px', top: 'auto' });
  });

  it('keeps a narrow window inside both horizontal margins', () => {
    const style = modelMenuPosition({ left: 20, top: 600, bottom: 630 }, 320, 720);
    expect(style).toMatchObject({ width: '304px', left: '8px' });
  });

  it('opens downwards near the top edge', () => {
    const style = modelMenuPosition({ left: 50, top: 30, bottom: 60 }, 900, 700);
    expect(style).toMatchObject({ left: '50px', top: '68px', bottom: 'auto', maxHeight: '624px' });
  });

  it('uses the larger available side in a short window', () => {
    const style = modelMenuPosition({ left: 600, top: 200, bottom: 230 }, 900, 360);
    expect(style).toMatchObject({ bottom: '168px', maxHeight: '184px' });
  });
});

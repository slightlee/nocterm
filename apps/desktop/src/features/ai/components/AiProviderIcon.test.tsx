import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { AI_PROVIDERS } from '../model/ai-types';
import { AiProviderIcon } from './AiProviderIcon';

describe('AiProviderIcon', () => {
  it('renders a distinct decorative icon for every registered provider', () => {
    const icons = AI_PROVIDERS.map(({ id }) =>
      renderToStaticMarkup(<AiProviderIcon className="provider-icon" providerId={id} />)
    );

    expect(new Set(icons).size).toBe(AI_PROVIDERS.length);
    icons.forEach((icon) => {
      expect(icon).toContain('aria-hidden="true"');
      expect(icon).toContain('class="provider-icon"');
    });
  });
});

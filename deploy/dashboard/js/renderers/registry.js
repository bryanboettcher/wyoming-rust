/**
 * Stage renderer registry.
 *
 * Maps stage names to specialized renderers. Falls back to a generic
 * key/value renderer for unknown stage types.
 */

import { AutoEnergyRenderer } from './auto_energy.js';
import { SpectrogramRenderer } from './spectrogram.js';
import { GenericRenderer } from './generic.js';

/**
 * Known stage name -> renderer constructor mappings.
 * Add new specialized renderers here.
 */
const KNOWN_RENDERERS = {
  auto_energy: AutoEnergyRenderer,
  mfcc: SpectrogramRenderer,
};

/**
 * Get a renderer instance for the given stage name.
 *
 * @param {string} stageName
 * @returns {{ create(name: string): HTMLElement, update(stats: object): void }}
 */
export function getRenderer(stageName) {
  const Ctor = KNOWN_RENDERERS[stageName];
  if (Ctor) return new Ctor();
  return new GenericRenderer();
}

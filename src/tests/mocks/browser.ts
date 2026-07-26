import { vi } from 'vitest';

class MockResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
}

class MockIntersectionObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
  takeRecords = vi.fn(() => []);
}

Object.defineProperty(window, 'ResizeObserver', {
  configurable: true,
  writable: true,
  value: MockResizeObserver,
});

Object.defineProperty(window, 'IntersectionObserver', {
  configurable: true,
  writable: true,
  value: MockIntersectionObserver,
});

Object.defineProperty(window, 'matchMedia', {
  configurable: true,
  writable: true,
  value: vi.fn((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

Object.defineProperty(window, 'requestAnimationFrame', {
  configurable: true,
  writable: true,
  value: vi.fn((callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0)),
});

Object.defineProperty(window, 'cancelAnimationFrame', {
  configurable: true,
  writable: true,
  value: vi.fn((id: number) => window.clearTimeout(id)),
});

Object.defineProperty(navigator, 'vibrate', {
  configurable: true,
  writable: true,
  value: vi.fn(),
});

Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    writeText: vi.fn(() => Promise.resolve()),
    readText: vi.fn(() => Promise.resolve('')),
  },
});

Object.defineProperty(URL, 'createObjectURL', {
  configurable: true,
  writable: true,
  value: vi.fn(() => 'blob:mock'),
});

Object.defineProperty(URL, 'revokeObjectURL', {
  configurable: true,
  writable: true,
  value: vi.fn(),
});

Object.defineProperty(window, 'visualViewport', {
  configurable: true,
  value: {
    height: 800,
    width: 400,
    offsetLeft: 0,
    offsetTop: 0,
    pageLeft: 0,
    pageTop: 0,
    scale: 1,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  },
});

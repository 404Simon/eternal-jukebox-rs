import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { defineConfig } from 'vitest/config'
import { playwright } from '@vitest/browser-playwright'
import react from '@vitejs/plugin-react'

// Prefer a system Chromium when one is available (e.g. NixOS). On CI none of
// these paths exist, so Playwright falls back to its own downloaded browser.
function systemChromium(): string | undefined {
  const candidates = [
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
    join(homedir(), '.nix-profile/bin/chromium'),
    '/run/current-system/sw/bin/chromium',
  ]
  return candidates.find((candidate): candidate is string => typeof candidate === 'string' && existsSync(candidate))
}

const executablePath = systemChromium()

export default defineConfig({
  plugins: [react()],
  worker: { format: 'es' },
  test: {
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(executablePath ? { launchOptions: { executablePath } } : {}),
      instances: [{ browser: 'chromium' }],
    },
  },
})

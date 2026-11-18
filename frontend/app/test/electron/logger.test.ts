// @vitest-environment node
// SPDX-License-Identifier: AGPL-3.0-only
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { Logger } from "../../electron/main/logging/logger"
import { mkdirSync, existsSync, rmSync, readFileSync } from "fs"
import { join } from "path"
import { mkdtempSync } from "fs"
import { tmpdir } from "os"

const LOG_HEADER_PREFIX = "=== Ferrofolio Electron Log Started at"

describe("Logger", () => {
  const tmpDir = mkdtempSync(join(tmpdir(), "logger-test-"))

  beforeEach(() => {
    vi.restoreAllMocks()
  })

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true })
  })

  it("writes the expected header into the main log file", async () => {
    const logDir = join(tmpDir, "logs")
    mkdirSync(logDir, { recursive: true })

    const logger = new Logger({ logDir })
    logger.initialize()

    const logFile = logger.getCurrentLogFile()
    expect(logFile).toBeDefined()
    if (logFile) {
      const content = readFileSync(logFile, "utf-8")
      expect(content.startsWith(LOG_HEADER_PREFIX)).toBe(true)
    }
  })

  it("rotates logs when the file exceeds max size", async () => {
    const logDir = join(tmpDir, "logs-rotate")
    mkdirSync(logDir, { recursive: true })

    const logger = new Logger({
      logDir,
      maxFileSize: 100,
      maxFiles: 2,
    })
    logger.initialize()

    const initialLog = logger.getCurrentLogFile()!
    expect(readFileSync(initialLog, "utf-8")).toContain(LOG_HEADER_PREFIX)

    // Write enough to trigger rotation
    for (let i = 0; i < 10; i++) {
      logger.info("x".repeat(80))
    }

    // After rotation the file should still exist
    expect(existsSync(initialLog)).toBe(true)
    const newContent = readFileSync(initialLog, "utf-8")
    expect(newContent).toContain(LOG_HEADER_PREFIX)
  })
})

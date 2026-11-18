// @vitest-environment node
// SPDX-License-Identifier: AGPL-3.0-only
import { afterEach, beforeEach, describe, expect, it } from "vitest"
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "fs"
import { join } from "path"
import { tmpdir } from "os"
import { Logger } from "../../electron/main/logging/logger"
import { getLogFilePath } from "../../electron/main/logging/utils"

const LOG_HEADER_PREFIX = "=== Ferrofolio Electron Log Started at"
const CONSOLE_METHODS = ["log", "info", "warn", "error", "debug"] as const

function requireLogFile(logger: Logger): string {
  const logFile = logger.getCurrentLogFile()
  if (!logFile) throw new Error("logger has no current log file")
  return logFile
}

describe("Logger", () => {
  let tmpDir: string
  let savedConsole: Pick<Console, (typeof CONSOLE_METHODS)[number]>

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "logger-test-"))
    savedConsole = Object.fromEntries(
      CONSOLE_METHODS.map(method => [method, console[method]]),
    ) as typeof savedConsole
  })

  afterEach(() => {
    // Logger.initialize() replaces these by assignment; restore them before the log directory goes away.
    for (const method of CONSOLE_METHODS) console[method] = savedConsole[method]
    rmSync(tmpDir, { recursive: true, force: true })
  })

  it("writes the expected header into the main log file", () => {
    const logDir = join(tmpDir, "logs")
    mkdirSync(logDir, { recursive: true })

    const logger = new Logger({ logDir })
    logger.initialize()

    const content = readFileSync(requireLogFile(logger), "utf-8")
    expect(content.startsWith(LOG_HEADER_PREFIX)).toBe(true)
  })

  it("archives the full log and starts a new one with the header", () => {
    const logDir = join(tmpDir, "logs-rotate")
    mkdirSync(logDir, { recursive: true })

    const logger = new Logger({ logDir, maxFileSize: 100, maxFiles: 2 })
    logger.initialize()
    logger.info("rotation-marker")
    logger.info("after rotation")

    const archived = readFileSync(getLogFilePath(logDir, 1), "utf-8")
    expect(archived).toContain("rotation-marker")

    const active = readFileSync(requireLogFile(logger), "utf-8")
    expect(requireLogFile(logger)).toBe(getLogFilePath(logDir, 0))
    expect(active.startsWith(LOG_HEADER_PREFIX)).toBe(true)
    expect(active).not.toContain("rotation-marker")
    expect(active).toContain("after rotation")
    expect(archived).not.toContain("after rotation")
  })
})

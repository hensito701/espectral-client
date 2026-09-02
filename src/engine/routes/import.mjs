// src/engine/routes/import.mjs — B3 import API.
//
//   GET  /api/import/sources            -> ImportSource[]
//   POST /api/instances/:name/import    -> ImportResult
//         body: { source_id, overwrite_policy: 'never'|'if-older' }
//
// Errors are thrown (httpError / ImportError) — server.mjs formats
// { error: { code, message } } with the matching status.

import path from 'node:path'
import { existsSync, readFileSync } from 'node:fs'
import { httpError } from '../error.mjs'
import { dataDir } from '../config.mjs'
import { detectImportSources, importProfile } from '../import.mjs'

const NAME_RE = /^[A-Za-z0-9 _-]{1,40}$/

/** Load an instance's persisted model; null when the instance does not exist. */
function loadInstance(name) {
  const dir = path.join(dataDir(), 'instances', name)
  const jsonPath = path.join(dir, 'instance.json')
  if (!existsSync(jsonPath)) return null
  try {
    const json = JSON.parse(readFileSync(jsonPath, 'utf8'))
    return { ...(json && typeof json === 'object' ? json : {}), name, dir }
  } catch {
    return null
  }
}

export async function register(app) {
  app.get('/api/import/sources', async () => {
    return await detectImportSources()
  })

  app.post('/api/instances/:name/import', async (req, res, params, body) => {
    const name = params && params.name
    if (typeof name !== 'string' || !NAME_RE.test(name)) {
      throw httpError(409, 'INVALID_NAME', `invalid instance name: ${String(name)}`)
    }
    const instance = loadInstance(name)
    if (!instance) throw httpError(404, 'NOT_FOUND', `instance "${name}" not found`)

    const payload = body && typeof body === 'object'
      ? body
      : req && req.body && typeof req.body === 'object' ? req.body : {}
    const sourceId = payload.source_id
    const overwritePolicy = payload.overwrite_policy ?? 'never'
    if (typeof sourceId !== 'string' || sourceId.length === 0) {
      throw httpError(400, 'BAD_REQUEST', 'source_id is required')
    }
    if (overwritePolicy !== 'never' && overwritePolicy !== 'if-older') {
      throw httpError(400, 'BAD_REQUEST', 'overwrite_policy must be "never" or "if-older"')
    }

    return await importProfile(instance, sourceId, overwritePolicy)
  })
}

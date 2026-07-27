#!/usr/bin/env node

import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
function argument(name) {
  const index = args.indexOf(name)
  if (index < 0 || !args[index + 1]) throw new Error(`${name} requires a value`)
  return args[index + 1]
}

const staticReport = JSON.parse(fs.readFileSync(argument('--static'), 'utf8'))
const requestLines = fs.readFileSync(argument('--requests'), 'utf8')
  .split('\n')
  .filter(Boolean)

function contractMatches(contractPath, requestPath) {
  const contractSegments = contractPath.split('/')
  const requestSegments = requestPath.split('/')
  if (contractSegments.length !== requestSegments.length) return false
  return contractSegments.every((segment, index) =>
    segment === '{param}' || segment === requestSegments[index],
  )
}

const requests = requestLines.map((line) => {
  const separator = line.indexOf('\t')
  if (separator < 0) return null
  const method = line.slice(0, separator).toUpperCase()
  const rawUrl = line.slice(separator + 1)
  let pathname
  try {
    pathname = new URL(rawUrl).pathname.replace(/\/+$/, '') || '/'
  } catch {
    return null
  }
  return { method, path: pathname }
}).filter(Boolean)

const contracts = staticReport.contracts.map((contract) => {
  const matchedRequests = requests.filter((request) =>
    request.method === contract.method && contractMatches(contract.path, request.path),
  )
  return {
    ...contract,
    runtime_request_count: matchedRequests.length,
    runtime_request_seen: matchedRequests.length > 0,
  }
})

const runtimeSeen = contracts.filter((contract) => contract.runtime_request_seen).length
const staticOnly = contracts.length - runtimeSeen
const report = {
  report_type: 'frontend_runtime_contract_coverage',
  limitation: 'Request coverage proves that the local runtime gate exercised the method/path. Response schemas, status codes, pagination, role checks, and tenant isolation are asserted by the named runtime matrix cases and are not inferred from a request trace.',
  generated_at_utc: new Date().toISOString(),
  frontend_method_path_contract_count: contracts.length,
  runtime_request_seen_count: runtimeSeen,
  static_only_count: staticOnly,
  contracts,
}
const output = argument('--output')
fs.mkdirSync(path.dirname(output), { recursive: true })
fs.writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`)
console.log(`Runtime request coverage: ${runtimeSeen}/${contracts.length}; static-only: ${staticOnly}`)

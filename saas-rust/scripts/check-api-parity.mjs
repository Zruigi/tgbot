import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const rustRoot = path.resolve(scriptDirectory, '..')
const repositoryRoot = path.resolve(rustRoot, '..')
const frontendRoot = path.join(repositoryRoot, 'frontend', 'src')
const rustRoutesFile = path.join(rustRoot, 'crates', 'http', 'src', 'routes', 'mod.rs')
const sourceExtensions = new Set(['.js', '.jsx', '.ts', '.tsx', '.vue'])
const reportArgumentIndex = process.argv.indexOf('--report')
const reportPath = reportArgumentIndex >= 0 ? process.argv[reportArgumentIndex + 1] : null
if (reportArgumentIndex >= 0 && !reportPath) {
  throw new Error('--report requires a file path')
}

function sourceFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) return sourceFiles(entryPath)
    return sourceExtensions.has(path.extname(entry.name)) ? [entryPath] : []
  })
}

function normalizePath(rawPath) {
  let routePath = rawPath
  const dynamicSuffix = routePath.indexOf('${')
  if (dynamicSuffix >= 0 && routePath[dynamicSuffix - 1] !== '/') {
    routePath = routePath.slice(0, dynamicSuffix)
  }
  routePath = routePath.replace(/\$\{[^}]*\}/g, (expression, offset) =>
    routePath[offset - 1] === '/' ? '{param}' : ''
  )
  routePath = routePath.split(/[?#]/, 1)[0]
  routePath = routePath.replace(/\/:([A-Za-z_][A-Za-z0-9_]*)/g, '/{param}')
  routePath = routePath.replace(/\{[^}]+\}/g, '{param}')
  routePath = routePath.replace(/:[A-Za-z_][A-Za-z0-9_]*/g, '{param}')
  return routePath.replace(/\/+$/, '') || '/'
}

function pathShape(routePath) {
  return normalizePath(routePath)
    .split('/')
    .map((segment) => (segment === '{param}' ? '*' : segment))
    .join('/')
}

const frontendPaths = new Set()
const frontendMethods = new Map()

function addFrontendMethod(routePath, method) {
  const normalized = normalizePath(routePath)
  frontendPaths.add(normalized)
  const methods = frontendMethods.get(normalized) || new Set()
  methods.add(method.toUpperCase())
  frontendMethods.set(normalized, methods)
}

for (const sourceFile of sourceFiles(frontendRoot)) {
  const source = fs.readFileSync(sourceFile, 'utf8')
  const apiConstants = new Map()

  for (const match of source.matchAll(/const\s+([A-Z][A-Z0-9_]*)\s*=\s*(['"`])(\/api\/[^'"`\s]+)\2/g)) {
    apiConstants.set(match[1], match[3])
  }

  // Only count paths used by request call sites. A raw `/api/...` text scan also
  // counted comments, error messages and base prefixes such as `/api/merchant`,
  // which made the old "94/95 paths" number larger than the real contract set.
  for (const [constantName, basePath] of apiConstants) {
    const referencePattern = new RegExp(
      `\\$\\{${constantName}\\}((?:/[A-Za-z0-9_.-]+|/\\$\\{[^}]+\\})*)`,
      'g'
    )
    for (const match of source.matchAll(referencePattern)) {
      const following = source.slice(match.index + match[0].length, match.index + match[0].length + 260)
      const method = following.match(/\bmethod\s*:\s*['"]([A-Za-z]+)['"]/)?.[1] || 'GET'
      addFrontendMethod(`${basePath}${match[1]}`, method)
    }
  }

  for (const match of source.matchAll(/fetch\(\s*([`'"])(\/api\/[^`'"\s]+)\1\s*(?:,\s*\{([\s\S]{0,800}?)\}\s*)?\)/g)) {
    const method = match[3]?.match(/\bmethod\s*:\s*['"]([A-Za-z]+)['"]/)?.[1] || 'GET'
    addFrontendMethod(match[2], method)
  }

  for (const match of source.matchAll(/\brequest\(\s*['"](GET|POST|PUT|PATCH|DELETE)['"]\s*,\s*([`'"])(\/api\/[^`'"]+)\2/g)) {
    addFrontendMethod(match[3], match[1])
  }

  for (const match of source.matchAll(/\b(?:apiJson|requestJson)\(\s*([`'"])(\/api\/[^`'"]+)\1\s*,\s*['"](GET|POST|PUT|PATCH|DELETE)['"]/g)) {
    addFrontendMethod(match[2], match[3])
  }

  for (const match of source.matchAll(/\bapiFormData\(\s*([`'"])(\/api\/[^`'"]+)\1/g)) {
    addFrontendMethod(match[2], 'POST')
  }

  // Resolve simple local endpoint variables, including the group batch-action
  // ternary (`const ep = condition ? '/a' : '/b'; fetch(ep, {method:'POST'})`).
  for (const match of source.matchAll(/\b(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=([\s\S]{0,400}?)(?:;|\n\s*(?=await\b))/g)) {
    const variableName = match[1]
    const expression = match[2]
    const paths = [...expression.matchAll(/([`'"])(\/api\/[^`'"\s]+)\1/g)].map((item) => item[2])
    if (paths.length === 0) continue
    const later = source.slice(match.index + match[0].length, match.index + match[0].length + 500)
    const callPattern = new RegExp(`\\bfetch\\(\\s*${variableName}\\b([\\s\\S]{0,350})`)
    const call = later.match(callPattern)
    if (!call) continue
    const method = call[1].match(/\bmethod\s*:\s*['"]([A-Za-z]+)['"]/)?.[1] || 'GET'
    for (const routePath of paths) addFrontendMethod(routePath, method)
  }
}

const rustSource = fs.readFileSync(rustRoutesFile, 'utf8')
const rustPaths = new Set(
  [...rustSource.matchAll(/"(\/api\/[^"\s]+)"/g)].map((match) => normalizePath(match[1]))
)
const rustShapes = new Set([...rustPaths].map(pathShape))
const rustMethods = new Map()
const routeMatches = [...rustSource.matchAll(/\.route\(\s*"(\/api\/[^"\s]+)"\s*,/g)]
for (const [index, match] of routeMatches.entries()) {
  const routePath = normalizePath(match[1])
  const end = routeMatches[index + 1]?.index ?? rustSource.length
  const routeExpression = rustSource.slice(match.index, end)
  const methods = rustMethods.get(pathShape(routePath)) || new Set()
  for (const methodMatch of routeExpression.matchAll(/\b(get|post|put|patch|delete)\s*\(/g)) {
    methods.add(methodMatch[1].toUpperCase())
  }
  rustMethods.set(pathShape(routePath), methods)
}
const missingPaths = [...frontendPaths]
  .filter((routePath) => {
    const shape = pathShape(routePath)
    return !rustShapes.has(shape) && ![...rustShapes].some((rustShape) => rustShape.startsWith(`${shape}/`))
  })
  .sort()
const missingMethods = []
for (const [routePath, methods] of frontendMethods) {
  const rustRouteMethods = rustMethods.get(pathShape(routePath)) || new Set()
  for (const method of methods) {
    if (!rustRouteMethods.has(method)) missingMethods.push(`${method} ${routePath}`)
  }
}
missingMethods.sort()
const frontendContractCount = [...frontendMethods.values()]
  .reduce((total, methods) => total + methods.size, 0)

console.log(`Frontend API paths: ${frontendPaths.size}`)
console.log(`Frontend API method/path contracts: ${frontendContractCount}`)
console.log(`Rust API paths: ${rustPaths.size}`)

if (process.env.API_PARITY_VERBOSE === '1') {
  for (const routePath of [...frontendPaths].sort()) {
    const methods = [...(frontendMethods.get(routePath) || [])].sort()
    console.log(`${methods.length > 0 ? methods.join(',') : 'UNKNOWN'} ${routePath}`)
  }
}

if (missingPaths.length > 0) {
  console.error('Missing Rust routes:')
  for (const routePath of missingPaths) console.error(`- ${routePath}`)
  process.exit(1)
}

if (missingMethods.length > 0) {
  console.error('Missing Rust route methods:')
  for (const routeMethod of missingMethods) console.error(`- ${routeMethod}`)
  process.exit(1)
}

console.log('All detected frontend API paths and methods are registered by the Rust router.')

if (reportPath) {
  const contracts = [...frontendMethods.entries()]
    .flatMap(([routePath, methods]) => [...methods].map((method) => ({
      method,
      path: routePath,
      rust_registered: (rustMethods.get(pathShape(routePath)) || new Set()).has(method),
    })))
    .sort((left, right) => `${left.path} ${left.method}`.localeCompare(`${right.path} ${right.method}`))
  const report = {
    report_type: 'frontend_rust_static_registration',
    limitation: 'Static registration only; response schemas, status codes, pagination, role checks, and tenant isolation require runtime contract evidence.',
    frontend_path_count: frontendPaths.size,
    frontend_method_path_contract_count: frontendContractCount,
    rust_path_count: rustPaths.size,
    missing_paths: missingPaths,
    missing_methods: missingMethods,
    contracts,
  }
  fs.mkdirSync(path.dirname(path.resolve(reportPath)), { recursive: true })
  fs.writeFileSync(path.resolve(reportPath), `${JSON.stringify(report, null, 2)}\n`)
  console.log(`Static registration report: ${path.resolve(reportPath)}`)
}

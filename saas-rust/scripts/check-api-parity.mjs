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

  for (const match of source.matchAll(/\/api\/[A-Za-z0-9_./-]*(?:\$\{[^}\n]+\}[A-Za-z0-9_./-]*)*/g)) {
    if (![...apiConstants.values()].includes(match[0])) {
      frontendPaths.add(normalizePath(match[0]))
    }
  }

  for (const [constantName, basePath] of apiConstants) {
    const referencePattern = new RegExp(
      `\\$\\{${constantName}\\}((?:/[A-Za-z0-9_.-]+|/\\$\\{[^}]+\\})*)`,
      'g'
    )
    for (const match of source.matchAll(referencePattern)) {
      frontendPaths.add(normalizePath(`${basePath}${match[1]}`))
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

console.log(`Frontend API paths: ${frontendPaths.size}`)
console.log(`Rust API paths: ${rustPaths.size}`)

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

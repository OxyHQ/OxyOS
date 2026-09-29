import { useState, useEffect } from 'react'

const DOWNLOAD_BASE = 'https://os.oxy.so/downloads'
const RELEASES_URL = 'https://os.oxy.so/packages/releases.json'
const ARCHS = ['amd64']

function compareVersions(a, b) {
  const pa = a.replace('v', '').split('.').map(Number)
  const pb = b.replace('v', '').split('.').map(Number)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const na = pa[i] || 0
    const nb = pb[i] || 0
    if (na !== nb) return nb - na
  }
  return 0
}

export function useReleases() {
  const [releases, setReleases] = useState([])
  const [latest, setLatest] = useState(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let ignore = false

    const cached = sessionStorage.getItem('oxyos-releases')
    if (cached) {
      const data = JSON.parse(cached)
      setReleases(data.releases)
      setLatest(data.latest)
      setLoading(false)
      return
    }

    async function fetchReleases() {
      try {
        const [releasesRes, tagsRes] = await Promise.all([
          fetch(RELEASES_URL),
          fetch('https://api.github.com/repos/OxyHQ/OxyOS/releases'),
        ])

        if (ignore) return

        const releasesData = await releasesRes.json()
        const ghReleases = await tagsRes.json()

        const currentVersion = releasesData.latest?.version || '1.0'
        const codename = releasesData.latest?.codename || 'Ozone'

        // Build list from GitHub Releases (published releases only, not old tags)
        const versions = Array.isArray(ghReleases)
          ? ghReleases
              .map(r => r.tag_name)
              .filter(n => /^v\d/.test(n))
              .sort(compareVersions)
          : []

        // Ensure the current version from releases.json is included
        const currentTag = `v${currentVersion}`
        if (!versions.includes(currentTag)) {
          versions.unshift(currentTag)
        }

        const latestVersion = versions[0] || currentTag
        const allDownloads = []

        versions.forEach((version, i) => {
          const ver = version.replace('v', '')
          ARCHS.forEach(arch => {
            allDownloads.push({
              version: `${version} (${codename})`,
              ver,
              arch,
              archLabel: 'amd64 (x86_64)',
              iso: `${DOWNLOAD_BASE}/oxyos-${ver}-${arch}.iso`,
              status: i === 0 ? 'Current' : 'Older',
            })
          })
        })

        if (!ignore) {
          setLatest(latestVersion)
          setReleases(allDownloads)
          setLoading(false)
          sessionStorage.setItem('oxyos-releases', JSON.stringify({
            releases: allDownloads,
            latest: latestVersion,
          }))
        }
      } catch {
        if (!ignore) setLoading(false)
      }
    }

    fetchReleases()
    return () => { ignore = true }
  }, [])

  return { releases, latest, loading }
}

export function getDownloadUrl(version, arch) {
  const ver = version ? version.replace('v', '') : '1.0'
  return `${DOWNLOAD_BASE}/oxyos-${ver}-${arch}.iso`
}

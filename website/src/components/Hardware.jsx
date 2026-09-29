import { Link } from 'react-router-dom'
import { useReleases, getDownloadUrl } from '../hooks/useReleases'

export default function Hardware() {
  const { latest } = useReleases()

  return (
    <div className="relative isolate bg-zinc-800 pb-32 pt-24 sm:pt-32">
      <div className="mx-auto max-w-7xl px-6 lg:px-8">
        <div className="mx-auto max-w-xl text-center">
          <h2 className="text-lg font-semibold leading-8 tracking-tight text-white">
            Supported Hardware
          </h2>
          <p className="mt-2 text-3xl font-bold tracking-tight text-zinc-300 sm:text-4xl">
            Runs on your machine
          </p>
        </div>
        <div className="mx-auto mt-16 grid max-w-2xl grid-cols-1 gap-8 text-sm leading-6 text-zinc-300 sm:mt-20 sm:grid-cols-2 xl:mx-0 xl:max-w-none xl:grid-cols-3">
          <div className="rounded-2xl bg-zinc-700 p-8 shadow-lg ring-1 ring-gray-900/5">
            <h3 className="text-lg font-semibold text-white">x86_64 / amd64</h3>
            <p className="mt-4 text-zinc-300">
              Standard desktop and laptop PCs with Intel or AMD processors.
              Full hardware support out of the box.
            </p>
            <ul className="mt-6 space-y-2 text-zinc-400">
              <li>Intel Core / AMD Ryzen</li>
              <li>BIOS and UEFI boot</li>
              <li>Full GPU support (Intel, AMD, NVIDIA)</li>
            </ul>
            <div className="mt-8">
              <a
                href={getDownloadUrl(latest, 'amd64')}
                className="rounded-md bg-stone-500 px-3.5 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-stone-400"
              >
                Download amd64
              </a>
            </div>
          </div>

          <div className="rounded-2xl bg-zinc-700 p-8 shadow-lg ring-1 ring-gray-900/5">
            <h3 className="text-lg font-semibold text-white">Minimum Requirements</h3>
            <p className="mt-4 text-zinc-300">
              OxyOS is designed to be lightweight. It runs well on modest hardware
              and flies on modern machines.
            </p>
            <ul className="mt-6 space-y-2 text-zinc-400">
              <li>1 GHz processor (64-bit)</li>
              <li>512 MB RAM (1 GB recommended)</li>
              <li>10 GB disk space</li>
              <li>USB port for live boot</li>
            </ul>
            <div className="mt-8">
              <Link
                to="/download"
                className="text-sm font-semibold text-white hover:text-zinc-400"
              >
                All downloads <span aria-hidden="true">&rarr;</span>
              </Link>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

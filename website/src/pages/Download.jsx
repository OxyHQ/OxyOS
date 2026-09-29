import Header from '../components/Header'
import Footer from '../components/Footer'
import { useReleases } from '../hooks/useReleases'

function DownloadIcon() {
  return (
    <svg
      className="h-5 w-5"
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      viewBox="0 0 24 24"
      strokeWidth="1.5"
      stroke="currentColor"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3"
      />
    </svg>
  )
}

export default function Download() {
  const { releases, loading } = useReleases()

  return (
    <div className="bg-zinc-900">
      <div className="relative isolate overflow-hidden bg-zinc-900">
        <svg
          className="absolute inset-0 -z-10 h-full w-full stroke-white/10 [mask-image:radial-gradient(100%_100%_at_top_right,white,transparent)]"
          aria-hidden="true"
        >
          <defs>
            <pattern
              id="983e3e4c-de6d-4c3f-8d64-b9761d1534cc"
              width="200"
              height="200"
              x="50%"
              y="-1"
              patternUnits="userSpaceOnUse"
            >
              <path d="M.5 200V.5H200" fill="none" />
            </pattern>
          </defs>
          <svg x="50%" y="-1" className="overflow-visible fill-zinc-800/20">
            <path
              d="M-200 0h201v201h-201Z M600 0h201v201h-201Z M-400 600h201v201h-201Z M200 800h201v201h-201Z"
              strokeWidth="0"
            />
          </svg>
          <rect
            width="100%"
            height="100%"
            strokeWidth="0"
            fill="url(#983e3e4c-de6d-4c3f-8d64-b9761d1534cc)"
          />
        </svg>
        <div
          className="absolute left-[calc(50%-4rem)] top-10 -z-10 transform-gpu blur-3xl sm:left-[calc(50%-18rem)] lg:left-48 lg:top-[calc(50%-30rem)] xl:left-[calc(50%-24rem)]"
          aria-hidden="true"
        />

        <Header />

        <div>
          <div className="mx-auto max-w-7xl">
            <div className="py-10">
              <div className="px-4 sm:px-6 lg:px-8">
                <div className="sm:flex sm:items-center">
                  <div className="sm:flex-auto">
                    <h1 className="text-base font-semibold leading-6 text-white text-xl">
                      Downloads
                    </h1>
                    <p className="mt-2 text-sm text-gray-300">
                      Download the latest version of OxyOS
                    </p>
                  </div>
                </div>

                <div className="mt-8 flow-root">
                  <div className="-mx-4 -my-2 overflow-x-auto sm:-mx-6 lg:-mx-8">
                    <div className="inline-block min-w-full py-2 align-middle sm:px-6 lg:px-8">
                      <div className="overflow-hidden shadow ring-1 ring-black ring-opacity-5 sm:rounded-lg">
                        <table className="min-w-full divide-y divide-gray-700">
                          <thead className="bg-zinc-700">
                            <tr>
                              <th
                                scope="col"
                                className="py-3.5 pl-4 pr-3 text-left text-sm font-semibold text-white sm:pl-6"
                              >
                                Version
                              </th>
                              <th
                                scope="col"
                                className="px-3 py-3.5 text-left text-sm font-semibold text-white"
                              >
                                Architecture
                              </th>
                              <th
                                scope="col"
                                className="px-3 py-3.5 text-left text-sm font-semibold text-white"
                              >
                                Direct Download
                              </th>
                              <th
                                scope="col"
                                className="px-3 py-3.5 text-left text-sm font-semibold text-white"
                              >
                                Status
                              </th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-zinc-700 bg-zinc-800">
                            {loading ? (
                              <tr>
                                <td colSpan="4" className="px-3 py-8 text-sm text-zinc-400 text-center">
                                  Loading releases...
                                </td>
                              </tr>
                            ) : releases.length === 0 ? (
                              <tr>
                                <td colSpan="4" className="px-3 py-8 text-sm text-zinc-400 text-center">
                                  No releases found
                                </td>
                              </tr>
                            ) : (
                              releases.map((dl) => (
                                <tr key={`${dl.ver}-${dl.arch}`}>
                                  <td className="whitespace-nowrap py-4 pl-4 pr-3 text-sm font-medium text-white sm:pl-6">
                                    {dl.version}
                                  </td>
                                  <td className="whitespace-nowrap px-3 py-4 text-sm text-zinc-300">
                                    {dl.archLabel}
                                  </td>
                                  <td className="whitespace-nowrap px-3 py-4 text-sm text-blue-300">
                                    <a
                                      href={dl.iso}
                                      className="flex items-center hover:text-gray-500"
                                    >
                                      <DownloadIcon />
                                      &nbsp;&nbsp; ISO
                                    </a>
                                  </td>
                                  <td className="whitespace-nowrap px-3 py-4 text-sm">
                                    <div
                                      className={`inline-flex items-center rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset ${
                                        dl.status === 'Current'
                                          ? 'bg-green-900 text-green-100 ring-green-100/20'
                                          : 'bg-zinc-700 text-zinc-300 ring-zinc-500/20'
                                      }`}
                                    >
                                      {dl.status}
                                    </div>
                                  </td>
                                </tr>
                              ))
                            )}
                          </tbody>
                        </table>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <Footer />
      </div>
    </div>
  )
}

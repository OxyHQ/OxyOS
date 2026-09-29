export default function FAQ() {
  return (
    <div className="bg-zinc-900 border-b border-zinc-700 drop-shadow-md" id="faq">
      <div className="mx-auto max-w-7xl px-6 py-24 sm:pt-32 lg:px-8 lg:py-40">
        <div className="lg:grid lg:grid-cols-12 lg:gap-8">
          <div className="lg:col-span-5">
            <h2 className="text-2xl font-bold leading-10 tracking-tight text-white">
              Frequently asked questions
            </h2>
            <p className="mt-4 text-base leading-7 text-zinc-300">
              Can't find the answer you're looking for? Reach out on our{' '}
              <a
                href="https://github.com/OxyHQ/OxyOS/issues"
                className="font-semibold text-gray-100 hover:text-gray-400"
              >
                GitHub issues
              </a>
              !
            </p>
          </div>
          <div className="mt-10 lg:col-span-7 lg:mt-0">
            <dl className="space-y-10">
              <div>
                <dt className="text-base font-semibold leading-7 text-white">
                  What are the login credentials for the live image?
                </dt>
                <dd className="mt-2 text-base leading-7 text-gray-300">
                  The username and password are both <b>'live'</b> without the
                  quotes.
                </dd>
              </div>
              <div>
                <dt className="text-base font-semibold leading-7 text-white">
                  What happened to the i686 (32-bit) image?
                </dt>
                <dd className="mt-2 text-base leading-7 text-gray-300">
                  The upstream base has dropped i686 as a first-class architecture.
                  While it is still possible to run a 32-bit userland on a 64-bit
                  kernel, we will no longer produce a 32-bit image.
                </dd>
              </div>
              <div>
                <dt className="text-base font-semibold leading-7 text-white">
                  Will you still be supporting older OxyOS releases?
                </dt>
                <dd className="mt-2 text-base leading-7 text-gray-300">
                  The upstream base continues to issue security updates for ~1 year after a
                  new stable release. While the older OxyOS releases won't
                  be getting any new updates from us, the repos will continue to
                  be available for at least the next year as well.
                </dd>
              </div>
              <div>
                <dt className="text-base font-semibold leading-7 text-white">
                  Where are the direct downloads?
                </dt>
                <dd className="mt-2 text-base leading-7 text-gray-300">
                  All older images are still available via Github Releases on the
                  image source{' '}
                  <a
                    className="font-semibold text-gray-100 hover:text-gray-400"
                    href="https://github.com/OxyHQ/OxyOS/releases"
                  >
                    Github repo
                  </a>
                  .
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </div>
    </div>
  )
}

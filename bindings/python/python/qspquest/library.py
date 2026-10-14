"""Discover shared models and run named scenarios through the verified loader."""
import json
import sys


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    try:
        from qspquest_models.cli import main as library_main
    except ImportError as error:
        fix = 'Install the companion model-library package from the matching release, then pass --library to its extracted directory.'
        if '--help' in argv or '-h' in argv:
            print(__doc__ + '\n' + fix + '\nRun --help again after installation for all supported options.')
            return
        payload = {'schema': 'qspquest.library-error/v1', 'code': 'loader_unavailable',
                   'problem': 'The model-library loader is unavailable', 'cause': str(error), 'fix': fix}
        message = (json.dumps(payload) if '--json-errors' in argv else
                   f"{payload['problem']} [loader_unavailable]: {error}\nFix: {fix}")
        print(message, file=sys.stderr)
        raise SystemExit(1)
    return library_main(argv, prog='qspquest-library')


if __name__ == '__main__':
    main()

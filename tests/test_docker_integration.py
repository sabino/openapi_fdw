import os
import shutil
import subprocess
import time
import unittest

try:
    import psycopg2
except Exception:  # pragma: no cover - dependency missing
    psycopg2 = None


class TestDockerPostgres(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if psycopg2 is None:
            raise unittest.SkipTest("psycopg2 not installed")
        if not shutil.which('docker'):
            raise unittest.SkipTest('docker not available')

        cls.openapi_spec = os.environ.get('OPENAPI_FDW_SPEC')
        cls.openapi_path = os.environ.get('OPENAPI_FDW_PATH')
        cls.openapi_columns = os.environ.get('OPENAPI_FDW_COLUMNS')
        if not all([cls.openapi_spec, cls.openapi_path, cls.openapi_columns]):
            raise unittest.SkipTest('OPENAPI_FDW_SPEC, OPENAPI_FDW_PATH, and OPENAPI_FDW_COLUMNS must be set')

        subprocess.run(['docker', 'build', '-t', 'openapi_fdw_test', '.'], check=True)
        cls.proc = subprocess.Popen(
            ['docker', 'run', '--rm', '-e', 'POSTGRES_PASSWORD=postgres', '-p', '55432:5432', 'openapi_fdw_test'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        for _ in range(30):
            try:
                conn = psycopg2.connect(host='localhost', port=55432, user='postgres', password='postgres', dbname='postgres')
                conn.close()
                break
            except Exception:
                time.sleep(1)
        else:
            cls.proc.terminate()
            cls.proc.wait()
            raise RuntimeError('postgres did not start')

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'proc'):
            cls.proc.terminate()
            cls.proc.wait()

    def test_query(self):
        conn = psycopg2.connect(host='localhost', port=55432, user='postgres', password='postgres', dbname='postgres')
        cur = conn.cursor()
        cur.execute('CREATE EXTENSION multicorn;')
        cur.execute(f"""
            CREATE SERVER openapi FOREIGN DATA WRAPPER multicorn OPTIONS (
                wrapper 'openapi_fdw.OpenAPIForeignDataWrapper',
                openapi_url '{self.openapi_spec}',
                path '{self.openapi_path}'
            );
        """)
        cur.execute("""
            CREATE FOREIGN TABLE items (
                {self.openapi_columns}
            ) SERVER openapi;
        """)
        cur.execute('SELECT * FROM items LIMIT 1;')
        row = cur.fetchone()
        self.assertIsNotNone(row)
        conn.close()


if __name__ == '__main__':
    unittest.main()

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
        if 'COINCAP_TOKEN' not in os.environ:
            raise unittest.SkipTest('COINCAP_TOKEN not set')

        subprocess.run(['docker', 'build', '-t', 'coincap_fdw_test', '.'], check=True)
        cls.proc = subprocess.Popen(
            ['docker', 'run', '--rm', '-e', 'POSTGRES_PASSWORD=postgres', '-p', '55432:5432', 'coincap_fdw_test'],
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
        token = os.environ['COINCAP_TOKEN']
        conn = psycopg2.connect(host='localhost', port=55432, user='postgres', password='postgres', dbname='postgres')
        cur = conn.cursor()
        cur.execute('CREATE EXTENSION multicorn;')
        cur.execute("""
            CREATE SERVER coincap FOREIGN DATA WRAPPER multicorn OPTIONS (
                wrapper 'coincap_fdw.CoinCapForeignDataWrapper',
                api_key '%s'
            );
        """ % token)
        cur.execute("""
            CREATE FOREIGN TABLE assets (
                id text,
                name text
            ) SERVER coincap;
        """)
        cur.execute('SELECT id FROM assets LIMIT 1;')
        row = cur.fetchone()
        self.assertIsNotNone(row)
        conn.close()


if __name__ == '__main__':
    unittest.main()

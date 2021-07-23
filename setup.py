import subprocess
from setuptools import setup, find_packages, Extension

setup(
  name='coincap_fdw',
  version='0.0.1',
  author='sabino',
  author_email='982190+sabino@users.noreply.github.com',
  url='https://github.com/sabino/coincap_fdw',
  license='WTFPL',
  packages=['coincap_fdw'],
  install_requires=[
      'requests==2.25.1',
      'hy==0.20.0'
  ],
  package_data={'coincap_fdw': ['*.hy']},
  package_dir={'coincap_fdw': 'src'}
)
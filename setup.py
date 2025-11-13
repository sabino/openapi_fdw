from setuptools import setup

setup(
    name="openapi_fdw",
    version="0.1.0",
    author="sabino",
    author_email="982190+sabino@users.noreply.github.com",
    url="https://github.com/sabino/openapi_fdw",
    license="WTFPL",
    packages=["openapi_fdw"],
    install_requires=[
        "requests>=2.31",
        "hy>=1.0",
    ],
    package_data={"openapi_fdw": ["*.py", "*.hy"]},
    package_dir={"openapi_fdw": "openapi_fdw"},
)

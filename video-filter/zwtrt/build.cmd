@echo off
rem Build zwtrt.exe with MSVC: build.cmd <TensorRT> <CUDA runtime>
rem Keep this file ASCII only (cmd misreads non-ASCII comment lines).
setlocal
if "%~2"=="" (echo build.cmd TensorRT CUDA& exit /b 2)
if not defined VCVARS for %%e in (Community Professional Enterprise BuildTools) do if exist "C:\Program Files\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=C:\Program Files\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat"
if not defined VCVARS for %%e in (BuildTools Community) do if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat"
call "%VCVARS%" >nul || exit /b 1
cd /d "%~dp0"
set "MORE="
if defined CUDA_PATH set MORE=/external:I"%CUDA_PATH%\include"
if not exist obj mkdir obj
set SOURCES=main.cpp server.cpp interpolator.cpp engine.cpp cuda_interop.cpp protocol.cpp report.cpp
rem TensorRT and CUDA headers are external: their warnings are not ours.
cl /nologo /std:c++20 /permissive- /W4 /O2 /EHsc /utf-8 /MP /external:W0 /external:I"%~1\include" /external:I"%~2\include" %MORE% /Fo"obj\\" /Fe:zwtrt.exe %SOURCES% /link /LIBPATH:"%~1\lib" /LIBPATH:"%~2\lib\x64" nvinfer_10.lib nvonnxparser_10.lib cudart.lib

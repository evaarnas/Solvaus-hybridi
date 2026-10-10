#include "Cube.h"

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <exception>
#include <mutex>
#include <new>
#include <vector>

struct RcubeMove {
	std::uint8_t face;
	std::uint32_t depth;
	std::int32_t turns;
};

using CancellationCheck = bool (*)(void*);
using ProgressObserver = void (*)(void*, double);

struct SolveContext {
	std::vector<RcubeMove> moves;
	CancellationCheck isCancelled;
	void* cancellationContext;
	ProgressObserver reportProgress;
	void* progressContext;
};

static std::mutex scrambleMutex;
static constexpr std::uint32_t MaxCubeSize = 255;

static void recordMove(void* rawContext, byte face, uint depth, int turns)
{
	auto* context = static_cast<SolveContext*>(rawContext);
	if (context->isCancelled != nullptr && context->isCancelled(context->cancellationContext))
	{
		throw CubeSolveCancelled{};
	}
	context->moves.push_back({face, depth, turns});
}

static void reportProgress(void* rawContext, double percent)
{
	auto* context = static_cast<SolveContext*>(rawContext);
	if (context->reportProgress != nullptr)
	{
		context->reportProgress(context->progressContext, percent);
	}
}

static void setError(char* error, std::size_t capacity, const char* message)
{
	if (error != nullptr && capacity > 0)
	{
		std::snprintf(error, capacity, "%s", message);
	}
}

extern "C" int rcube_solve(
	std::uint32_t size,
	const std::uint8_t* colors,
	std::size_t colorCount,
	CancellationCheck isCancelled,
	void* cancellationContext,
	ProgressObserver progressObserver,
	void* progressContext,
	RcubeMove** outputMoves,
	std::size_t* outputCount,
	char* error,
	std::size_t errorCapacity)
{
	if (outputMoves == nullptr || outputCount == nullptr)
	{
		setError(error, errorCapacity, "missing output buffer");
		return 1;
	}
	*outputMoves = nullptr;
	*outputCount = 0;

	if (size < 4 || size > MaxCubeSize || colors == nullptr ||
		colorCount != static_cast<std::size_t>(6) * size * size)
	{
		setError(error, errorCapacity, "RCube supports a complete 4x4 through 255x255 sticker state");
		return 1;
	}

	try
	{
		Cube cube(static_cast<uint>(size));
		cube.MovesPerFrame = 0;

		// Cube Solver order: U,D,F,B,L,R. RCube order: F,R,B,L,U,D.
		static constexpr std::uint8_t inputFaceForRcubeFace[6] = {2, 5, 3, 4, 0, 1};
		// Cube Solver colors: W,Y,G,B,O,R. RCube uses its solved face IDs.
		static constexpr std::uint8_t rcubeColorForInputColor[6] = {4, 5, 0, 2, 3, 1};
		const std::size_t faceArea = static_cast<std::size_t>(size) * size;
		for (std::uint8_t face = 0; face < 6; ++face)
		{
			const std::size_t inputFace = inputFaceForRcubeFace[face];
			for (std::uint32_t row = 0; row < size; ++row)
			{
				for (std::uint32_t col = 0; col < size; ++col)
				{
					// RCube's face rows run opposite to cube_core's.
					const auto inputRow = size - 1 - row;
					const auto color = colors[inputFace * faceArea + inputRow * size + col];
					if (color >= 6)
					{
						setError(error, errorCapacity, "sticker color index is out of range");
						return 1;
					}
					cube.faces[face].SetRC(row, col, rcubeColorForInputColor[color]);
				}
			}
		}

		SolveContext context{{}, isCancelled, cancellationContext, progressObserver, progressContext};
		cube.SetMoveObserver(recordMove, &context);
		cube.SetProgressObserver(reportProgress, &context);
		cube.Solve();
		cube.SetMoveObserver(nullptr, nullptr);
		cube.SetProgressObserver(nullptr, nullptr);

		if (!cube.IsCubeSolved())
		{
			setError(error, errorCapacity, "RCube finished without solving the cube");
			return 1;
		}
		reportProgress(&context, 100.0);

		if (!context.moves.empty())
		{
			auto* result = new RcubeMove[context.moves.size()];
			std::copy(context.moves.begin(), context.moves.end(), result);
			*outputMoves = result;
		}
		*outputCount = context.moves.size();
		return 0;
	}
	catch (const CubeSolveCancelled&)
	{
		setError(error, errorCapacity, "solve cancelled or timed out");
		return 2;
	}
	catch (const std::bad_alloc&)
	{
		setError(error, errorCapacity, "RCube ran out of memory");
		return 1;
	}
	catch (const std::exception& exception)
	{
		setError(error, errorCapacity, exception.what());
		return 1;
	}
	catch (...)
	{
		setError(error, errorCapacity, "RCube failed with an unknown error");
		return 1;
	}
}

extern "C" int rcube_scramble(
	std::uint32_t size,
	std::int32_t seed,
	RcubeMove** outputMoves,
	std::size_t* outputCount,
	char* error,
	std::size_t errorCapacity)
{
	if (outputMoves == nullptr || outputCount == nullptr)
	{
		setError(error, errorCapacity, "missing output buffer");
		return 1;
	}
	*outputMoves = nullptr;
	*outputCount = 0;

	if (size < 4 || size > MaxCubeSize)
	{
		setError(error, errorCapacity, "RCube supports scrambling 4x4 through 255x255");
		return 1;
	}

	try
	{
		Cube cube(static_cast<uint>(size));
		cube.MovesPerFrame = 0;
		SolveContext context{{}, nullptr, nullptr, nullptr, nullptr};
		cube.SetMoveObserver(recordMove, &context);
		{
			std::lock_guard<std::mutex> lock(scrambleMutex);
			cube.Scramble(seed);
		}
		cube.SetMoveObserver(nullptr, nullptr);

		auto* result = new RcubeMove[context.moves.size()];
		std::copy(context.moves.begin(), context.moves.end(), result);
		*outputMoves = result;
		*outputCount = context.moves.size();
		return 0;
	}
	catch (const std::bad_alloc&)
	{
		setError(error, errorCapacity, "RCube ran out of memory");
		return 1;
	}
	catch (const std::exception& exception)
	{
		setError(error, errorCapacity, exception.what());
		return 1;
	}
	catch (...)
	{
		setError(error, errorCapacity, "RCube failed with an unknown error");
		return 1;
	}
}

extern "C" void rcube_free_moves(RcubeMove* moves)
{
	delete[] moves;
}

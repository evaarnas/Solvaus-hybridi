#include "Cube.h"

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <exception>
#include <new>
#include <vector>

struct RcubeMove {
	std::uint8_t face;
	std::uint32_t depth;
	std::int32_t turns;
};

using CancellationCheck = bool (*)(void*);

struct SolveContext {
	std::vector<RcubeMove> moves;
	CancellationCheck isCancelled;
	void* cancellationContext;
};

static void recordMove(void* rawContext, byte face, uint depth, int turns)
{
	auto* context = static_cast<SolveContext*>(rawContext);
	if (context->isCancelled != nullptr && context->isCancelled(context->cancellationContext))
	{
		throw CubeSolveCancelled{};
	}
	context->moves.push_back({face, depth, turns});
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

	if (size < 4 || size > 11 || colors == nullptr ||
		colorCount != static_cast<std::size_t>(6) * size * size)
	{
		setError(error, errorCapacity, "RCube supports a complete 4x4 through 11x11 sticker state");
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
					const auto color = colors[inputFace * faceArea + row * size + col];
					if (color >= 6)
					{
						setError(error, errorCapacity, "sticker color index is out of range");
						return 1;
					}
					cube.faces[face].SetRC(row, col, rcubeColorForInputColor[color]);
				}
			}
		}

		SolveContext context{{}, isCancelled, cancellationContext};
		cube.SetMoveObserver(recordMove, &context);
		cube.Solve();
		cube.SetMoveObserver(nullptr, nullptr);

		if (!cube.IsCubeSolved())
		{
			setError(error, errorCapacity, "RCube finished without solving the cube");
			return 1;
		}

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

extern "C" void rcube_free_moves(RcubeMove* moves)
{
	delete[] moves;
}

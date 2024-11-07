// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract OptimizedArbitrage {
    function executeArbitrage(
        address[] calldata path,
        uint256 amountIn
    ) external payable {
        // Direct pool interactions
        uint256 currentAmount = amountIn;

        for (uint i = 0; i < path.length - 1; i++) {
            IUniswapV2Pair pair = IUniswapV2Pair(pools[i]);
            currentAmount = _executeSwap(
                pair,
                path[i],
                path[i + 1],
                currentAmount
            );
        }

        // Transfer profit
        require(currentAmount > amountIn, "No profit");
        TransferHelper.safeTransfer(path[0], msg.sender, currentAmount);
    }

    function _executeSwap(
        IUniswapV2Pair pair,
        address tokenIn,
        address tokenOut,
        uint256 amountIn
    ) internal returns (uint256) {
        (uint256 amount0Out, uint256 amount1Out) = pair.token0() == tokenOut
            ? (amountIn, 0)
            : (0, amountIn);

        pair.swap(amount0Out, amount1Out, address(this), new bytes(0));
    }
}
